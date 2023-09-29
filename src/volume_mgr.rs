//! The Volume Manager implementation.
//!
//! The volume manager handles partitions and open files on a block device.

use core::convert::TryFrom;
use core::ops::ControlFlow;

use crate::fat::{self, BlockCache, RESERVED_ENTRIES};

use crate::filesystem::{
    Attributes, ClusterId, DirEntry, Directory, DirectoryInfo, File, FileInfo, Mode,
    SearchIdGenerator, TimeSource, ToShortFileName, MAX_FILE_SIZE,
};
use crate::{debug, Block, BlockCount, BlockDevice, BlockIdx, Error, VolumeType};
use heapless::Vec;

/// A `VolumeManager` wraps a block device and gives access to the FAT-formatted
/// volumes within it.
pub struct VolumeManager<D, T, const MAX_DIRS: usize = 4, const MAX_FILES: usize = 4>
where
    D: BlockDevice,
    T: TimeSource,
    <D as BlockDevice>::Error: core::fmt::Debug,
{
    pub(crate) block_device: D,
    pub(crate) time_source: T,
    id_generator: SearchIdGenerator,
    volume_type: VolumeType,
    open_dirs: Vec<DirectoryInfo, MAX_DIRS>,
    open_files: Vec<FileInfo, MAX_FILES>,
}

impl<D, T> VolumeManager<D, T, 4, 4>
where
    D: BlockDevice,
    T: TimeSource,
    <D as BlockDevice>::Error: core::fmt::Debug,
{
    /// Create a new Volume Manager using a generic `BlockDevice`. From this
    /// object we can open volumes (partitions) and with those we can open
    /// files.
    ///
    /// This creates a `VolumeManager` with default values
    /// MAX_DIRS = 4, MAX_FILES = 4, MAX_VOLUMES = 1. Call `VolumeManager::new_with_limits(block_device, time_source)`
    /// if you need different limits.
    pub async fn new(
        block_device: D,
        time_source: T,
    ) -> Result<VolumeManager<D, T, 4, 4>, Error<D::Error>> {
        // Pick a random starting point for the IDs that's not zero, because
        // zero doesn't stand out in the logs.
        Self::new_with_limits(block_device, time_source, 5000).await
    }
}

impl<D, T, const MAX_DIRS: usize, const MAX_FILES: usize> VolumeManager<D, T, MAX_DIRS, MAX_FILES>
where
    D: BlockDevice,
    T: TimeSource,
    <D as BlockDevice>::Error: core::fmt::Debug,
{
    /// Create a new Volume Manager using a generic `BlockDevice`. From this
    /// object we can open volumes (partitions) and with those we can open
    /// files.
    ///
    /// You can also give an offset for all the IDs this volume manager
    /// generates, which might help you find the IDs in your logs when
    /// debugging.
    pub async fn new_with_limits(
        block_device: D,
        time_source: T,
        id_offset: u32,
    ) -> Result<VolumeManager<D, T, MAX_DIRS, MAX_FILES>, Error<D::Error>> {
        debug!("Creating new embedded-sdmmc::VolumeManager");
        let volume_type =
            fat::parse_volume(&block_device, BlockIdx(0), block_device.num_blocks().await?).await?;
        Ok(VolumeManager {
            block_device,
            time_source,
            id_generator: SearchIdGenerator::new(id_offset),
            volume_type,
            open_dirs: Vec::new(),
            open_files: Vec::new(),
        })
    }

    /// Temporarily get access to the underlying block device.
    pub fn device(&mut self) -> &mut D {
        &mut self.block_device
    }

    /// Open the volume's root directory.
    ///
    /// You can then read the directory entries with `iterate_dir`, or you can
    /// use `open_file_in_dir`.
    pub fn open_root_dir(&mut self) -> Result<Directory, Error<D::Error>> {
        for dir in self.open_dirs.iter() {
            if dir.cluster == ClusterId::ROOT_DIR {
                return Err(Error::DirAlreadyOpen);
            }
        }

        let directory_id = Directory(self.id_generator.get());
        let dir_info = DirectoryInfo {
            cluster: ClusterId::ROOT_DIR,
            directory_id,
        };

        self.open_dirs
            .push(dir_info)
            .map_err(|_| Error::TooManyOpenDirs)?;

        Ok(directory_id)
    }

    /// Open a directory.
    ///
    /// You can then read the directory entries with `iterate_dir` and `open_file_in_dir`.
    ///
    /// TODO: Work out how to prevent damage occuring to the file system while
    /// this directory handle is open. In particular, stop this directory
    /// being unlinked.
    pub async fn open_dir<N>(
        &mut self,
        parent_dir: Directory,
        name: N,
    ) -> Result<Directory, Error<D::Error>>
    where
        N: ToShortFileName,
    {
        if self.open_dirs.is_full() {
            return Err(Error::TooManyOpenDirs);
        }

        // Find dir by ID
        let parent_dir_idx = self.get_dir_by_id(parent_dir)?;
        let short_file_name = name.to_short_filename().map_err(Error::FilenameError)?;

        // Open the directory
        let parent_dir_info = &self.open_dirs[parent_dir_idx];
        let dir_entry = match &self.volume_type {
            VolumeType::Fat(fat) => {
                fat.find_directory_entry(&self.block_device, parent_dir_info, &short_file_name)
                    .await?
            }
        };

        if !dir_entry.attributes.is_directory() {
            return Err(Error::OpenedFileAsDir);
        }

        // Check it's not already open
        for d in self.open_dirs.iter() {
            if d.cluster == dir_entry.cluster {
                return Err(Error::DirAlreadyOpen);
            }
        }

        // Remember this open directory.
        let directory_id = Directory(self.id_generator.get());
        let dir_info = DirectoryInfo {
            directory_id,
            cluster: dir_entry.cluster,
        };

        self.open_dirs
            .push(dir_info)
            .map_err(|_| Error::TooManyOpenDirs)?;

        Ok(directory_id)
    }

    /// Close a directory. You cannot perform operations on an open directory
    /// and so must close it if you want to do something with it.
    pub fn close_dir(&mut self, directory: Directory) -> Result<(), Error<D::Error>> {
        for (idx, info) in self.open_dirs.iter().enumerate() {
            if directory == info.directory_id {
                self.open_dirs.swap_remove(idx);
                return Ok(());
            }
        }
        Err(Error::BadHandle)
    }

    /// Look in a directory for a named file.
    pub async fn find_directory_entry<N>(
        &mut self,
        directory: Directory,
        name: N,
    ) -> Result<DirEntry, Error<D::Error>>
    where
        N: ToShortFileName,
    {
        let directory_idx = self.get_dir_by_id(directory)?;
        match &self.volume_type {
            VolumeType::Fat(fat) => {
                let sfn = name.to_short_filename().map_err(Error::FilenameError)?;
                fat.find_directory_entry(&self.block_device, &self.open_dirs[directory_idx], &sfn)
                    .await
            }
        }
    }

    #[allow(missing_docs)]
    pub async fn find_lfn_directory_entry(
        &mut self,
        directory: Directory,
        name: &str,
    ) -> Result<DirEntry, Error<D::Error>> {
        let directory_idx = self.get_dir_by_id(directory)?;
        match &self.volume_type {
            VolumeType::Fat(fat) => {
                fat.find_lfn_directory_entry(
                    &self.block_device,
                    &self.open_dirs[directory_idx],
                    &name,
                )
                .await
            }
        }
    }

    /// Call a callback function for each directory entry in a directory.
    pub async fn iterate_dir<F, U>(
        &mut self,
        directory: Directory,
        func: F,
    ) -> Result<Option<U>, Error<D::Error>>
    where
        F: FnMut(&DirEntry) -> ControlFlow<U>,
    {
        let directory_idx = self.get_dir_by_id(directory)?;
        match &self.volume_type {
            VolumeType::Fat(fat) => {
                fat.iterate_dir(&self.block_device, &self.open_dirs[directory_idx], func)
                    .await
            }
        }
    }

    /// Call a callback function for each directory entry in a directory, with its LFN if it has one.
    pub async fn iterate_lfn_dir<F, U>(
        &mut self,
        directory: Directory,
        func: F,
    ) -> Result<Option<U>, Error<D::Error>>
    where
        F: FnMut(Option<&str>, &DirEntry) -> ControlFlow<U>,
    {
        let directory_idx = self.get_dir_by_id(directory)?;
        match &self.volume_type {
            VolumeType::Fat(fat) => {
                fat.iterate_lfn_dir(&self.block_device, &self.open_dirs[directory_idx], func)
                    .await
            }
        }
    }

    /// Open a file from a DirEntry. This is obtained by calling iterate_dir.
    ///
    /// # Safety
    ///
    /// The DirEntry must be a valid DirEntry read from disk, and not just
    /// random numbers.
    async unsafe fn open_dir_entry(
        &mut self,
        dir_entry: DirEntry,
        mode: Mode,
    ) -> Result<File, Error<D::Error>> {
        // This check is load-bearing - we do an unchecked push later.
        if self.open_files.is_full() {
            return Err(Error::TooManyOpenFiles);
        }

        if dir_entry.attributes.is_read_only() && mode != Mode::ReadOnly {
            return Err(Error::ReadOnly);
        }

        if dir_entry.attributes.is_directory() {
            return Err(Error::OpenedDirAsFile);
        }

        // Check it's not already open
        if self.file_is_open(&dir_entry) {
            return Err(Error::FileAlreadyOpen);
        }

        let mode = solve_mode_variant(mode, true);
        let file_id = File(self.id_generator.get());

        let file = match mode {
            Mode::ReadOnly => FileInfo {
                file_id,
                current_cluster: (0, dir_entry.cluster),
                current_offset: 0,
                mode,
                entry: dir_entry,
                dirty: false,
            },
            Mode::ReadWriteAppend => {
                let mut file = FileInfo {
                    file_id,
                    current_cluster: (0, dir_entry.cluster),
                    current_offset: 0,
                    mode,
                    entry: dir_entry,
                    dirty: false,
                };
                // seek_from_end with 0 can't fail
                file.seek_from_end(0).ok();
                file
            }
            Mode::ReadWriteTruncate => {
                let mut file = FileInfo {
                    file_id,
                    current_cluster: (0, dir_entry.cluster),
                    current_offset: 0,
                    mode,
                    entry: dir_entry,
                    dirty: false,
                };
                match &mut self.volume_type {
                    VolumeType::Fat(fat) => {
                        fat.truncate_cluster_chain(&self.block_device, file.entry.cluster)
                            .await?
                    }
                };
                file.update_length(0);
                match &self.volume_type {
                    VolumeType::Fat(fat) => {
                        file.entry.mtime = self.time_source.get_timestamp();
                        let fat_type = fat.get_fat_type();
                        self.write_entry_to_disk(fat_type, &file.entry).await?;
                    }
                };

                file
            }
            _ => return Err(Error::Unsupported),
        };

        // Remember this open file - can't be full as we checked already
        unsafe {
            self.open_files.push_unchecked(file);
        }

        Ok(file_id)
    }

    /// Open a file with the given full path. A file can only be opened once.
    pub async fn open_file_in_dir<N>(
        &mut self,
        directory: Directory,
        name: N,
        mode: Mode,
    ) -> Result<File, Error<D::Error>>
    where
        N: ToShortFileName,
    {
        // This check is load-bearing - we do an unchecked push later.
        if self.open_files.is_full() {
            return Err(Error::TooManyOpenFiles);
        }

        let directory_idx = self.get_dir_by_id(directory)?;
        let directory_info = &self.open_dirs[directory_idx];
        let sfn = name.to_short_filename().map_err(Error::FilenameError)?;

        let dir_entry = match &self.volume_type {
            VolumeType::Fat(fat) => {
                fat.find_directory_entry(&self.block_device, directory_info, &sfn)
                    .await
            }
        };

        let dir_entry = match dir_entry {
            Ok(entry) => {
                // we are opening an existing file
                Some(entry)
            }
            Err(_)
                if (mode == Mode::ReadWriteCreate)
                    | (mode == Mode::ReadWriteCreateOrTruncate)
                    | (mode == Mode::ReadWriteCreateOrAppend) =>
            {
                // We are opening a non-existant file, but that's OK because they
                // asked us to create it
                None
            }
            _ => {
                // We are opening a non-existant file, and that's not OK.
                return Err(Error::FileNotFound);
            }
        };

        // Check if it's open already
        if let Some(dir_entry) = &dir_entry {
            if self.file_is_open(&dir_entry) {
                return Err(Error::FileAlreadyOpen);
            }
        }

        let mode = solve_mode_variant(mode, dir_entry.is_some());

        match mode {
            Mode::ReadWriteCreate => {
                if dir_entry.is_some() {
                    return Err(Error::FileAlreadyExists);
                }
                let att = Attributes::create_from_fat(0);
                let entry = match &mut self.volume_type {
                    VolumeType::Fat(fat) => {
                        fat.write_new_directory_entry(
                            &self.block_device,
                            &self.time_source,
                            directory_info,
                            sfn,
                            att,
                        )
                        .await?
                    }
                };

                let file_id = File(self.id_generator.get());

                let file = FileInfo {
                    file_id,
                    current_cluster: (0, entry.cluster),
                    current_offset: 0,
                    mode,
                    entry,
                    dirty: false,
                };

                // Remember this open file - can't be full as we checked already
                unsafe {
                    self.open_files.push_unchecked(file);
                }

                Ok(file_id)
            }
            _ => {
                // Safe to unwrap, since we actually have an entry if we got here
                let dir_entry = dir_entry.unwrap();
                // Safety: We read this dir entry off disk and didn't change it
                unsafe { self.open_dir_entry(dir_entry, mode).await }
            }
        }
    }

    /// Delete a closed file with the given filename, if it exists.
    pub async fn delete_file_in_dir<N>(
        &mut self,
        directory: Directory,
        name: N,
    ) -> Result<(), Error<D::Error>>
    where
        N: ToShortFileName,
    {
        let dir_idx = self.get_dir_by_id(directory)?;
        let dir_info = &self.open_dirs[dir_idx];
        let sfn = name.to_short_filename().map_err(Error::FilenameError)?;

        let dir_entry = match &self.volume_type {
            VolumeType::Fat(fat) => {
                fat.find_directory_entry(&self.block_device, dir_info, &sfn)
                    .await
            }
        }?;

        if dir_entry.attributes.is_directory() {
            return Err(Error::DeleteDirAsFile);
        }

        if self.file_is_open(&dir_entry) {
            return Err(Error::FileAlreadyOpen);
        }

        match &self.volume_type {
            VolumeType::Fat(fat) => {
                fat.delete_directory_entry(&self.block_device, dir_info, &sfn)
                    .await?
            }
        }

        Ok(())
    }

    /// Check if a file is open
    ///
    /// Returns `true` if it's open, `false`, otherwise.
    fn file_is_open(&self, dir_entry: &DirEntry) -> bool {
        for f in self.open_files.iter() {
            if f.entry.entry_block == dir_entry.entry_block
                && f.entry.entry_offset == dir_entry.entry_offset
            {
                return true;
            }
        }
        false
    }

    /// Read from an open file.
    pub async fn read(&mut self, file: File, buffer: &mut [u8]) -> Result<usize, Error<D::Error>> {
        let file_idx = self.get_file_by_id(file)?;
        // Calculate which file block the current offset lies within
        // While there is more to read, read the block and copy in to the buffer.
        // If we need to find the next cluster, walk the FAT.
        let mut space = buffer.len();
        let mut read = 0;
        while space > 0 && !self.open_files[file_idx].eof() {
            let mut current_cluster = self.open_files[file_idx].current_cluster;
            let (block_idx, block_offset, block_avail) = self
                .find_data_on_disk(
                    &mut current_cluster,
                    self.open_files[file_idx].current_offset,
                )
                .await?;
            self.open_files[file_idx].current_cluster = current_cluster;
            let mut blocks = [Block::new()];
            self.block_device
                .read(&mut blocks, block_idx, "read")
                .await
                .map_err(Error::DeviceError)?;
            let block = &blocks[0];
            let to_copy = block_avail
                .min(space)
                .min(self.open_files[file_idx].left() as usize);
            assert!(to_copy != 0);
            buffer[read..read + to_copy]
                .copy_from_slice(&block[block_offset..block_offset + to_copy]);
            read += to_copy;
            space -= to_copy;
            self.open_files[file_idx]
                .seek_from_current(to_copy as i32)
                .unwrap();
        }
        Ok(read)
    }

    /// Write to a open file.
    pub async fn write(&mut self, file: File, buffer: &[u8]) -> Result<usize, Error<D::Error>> {
        #[cfg(feature = "defmt-log")]
        debug!("write(file={:?}, buffer={:x}", file, buffer);

        #[cfg(feature = "log")]
        debug!("write(file={:?}, buffer={:x?}", file, buffer);

        // Clone this so we can touch our other structures. Need to ensure we
        // write it back at the end.
        let file_idx = self.get_file_by_id(file)?;

        if self.open_files[file_idx].mode == Mode::ReadOnly {
            return Err(Error::ReadOnly);
        }

        self.open_files[file_idx].dirty = true;

        if self.open_files[file_idx].entry.cluster.0 < RESERVED_ENTRIES {
            // file doesn't have a valid allocated cluster (possible zero-length file), allocate one
            self.open_files[file_idx].entry.cluster = match self.volume_type {
                VolumeType::Fat(ref mut fat) => {
                    fat.alloc_cluster(&self.block_device, None, false).await?
                }
            };
            debug!(
                "Alloc first cluster {:?}",
                self.open_files[file_idx].entry.cluster
            );
        }

        if (self.open_files[file_idx].current_cluster.1) < self.open_files[file_idx].entry.cluster {
            debug!("Rewinding to start");
            self.open_files[file_idx].current_cluster =
                (0, self.open_files[file_idx].entry.cluster);
        }
        let bytes_until_max =
            usize::try_from(MAX_FILE_SIZE - self.open_files[file_idx].current_offset)
                .map_err(|_| Error::ConversionError)?;
        let bytes_to_write = core::cmp::min(buffer.len(), bytes_until_max);
        let mut written = 0;

        while written < bytes_to_write {
            let mut current_cluster = self.open_files[file_idx].current_cluster;
            debug!(
                "Have written bytes {}/{}, finding cluster {:?}",
                written, bytes_to_write, current_cluster
            );
            let current_offset = self.open_files[file_idx].current_offset;
            let (block_idx, block_offset, block_avail) = match self
                .find_data_on_disk(&mut current_cluster, current_offset)
                .await
            {
                Ok(vars) => {
                    debug!(
                        "Found block_idx={:?}, block_offset={:?}, block_avail={}",
                        vars.0, vars.1, vars.2
                    );
                    vars
                }
                Err(Error::EndOfFile) => {
                    debug!("Extending file");
                    match self.volume_type {
                        VolumeType::Fat(ref mut fat) => {
                            if fat
                                .alloc_cluster(&self.block_device, Some(current_cluster.1), false)
                                .await
                                .is_err()
                            {
                                return Ok(written);
                            }
                            debug!("Allocated new FAT cluster, finding offsets...");
                            let new_offset = self
                                .find_data_on_disk(
                                    &mut current_cluster,
                                    self.open_files[file_idx].current_offset,
                                )
                                .await
                                .map_err(|_| Error::AllocationError)?;
                            debug!("New offset {:?}", new_offset);
                            new_offset
                        }
                    }
                }
                Err(e) => return Err(e),
            };
            let mut blocks = [Block::new()];
            let to_copy = core::cmp::min(block_avail, bytes_to_write - written);
            if block_offset != 0 {
                debug!("Partial block write");
                self.block_device
                    .read(&mut blocks, block_idx, "read")
                    .await
                    .map_err(Error::DeviceError)?;
            }
            let block = &mut blocks[0];
            block[block_offset..block_offset + to_copy]
                .copy_from_slice(&buffer[written..written + to_copy]);
            debug!("Writing block {:?}", block_idx);
            self.block_device
                .write(&blocks, block_idx)
                .await
                .map_err(Error::DeviceError)?;
            written += to_copy;
            self.open_files[file_idx].current_cluster = current_cluster;

            let to_copy = to_copy as u32;
            let new_offset = self.open_files[file_idx].current_offset + to_copy;
            if new_offset > self.open_files[file_idx].entry.size {
                // We made it longer
                self.open_files[file_idx].update_length(new_offset);
            }
            self.open_files[file_idx]
                .seek_from_start(new_offset)
                .unwrap();
            // Entry update deferred to file close, for performance.
        }
        self.open_files[file_idx].entry.attributes.set_archive(true);
        self.open_files[file_idx].entry.mtime = self.time_source.get_timestamp();
        Ok(written)
    }

    /// Close a file with the given full path.
    pub async fn close_file(&mut self, file: File) -> Result<(), Error<D::Error>> {
        let mut found_idx = None;
        for (idx, info) in self.open_files.iter().enumerate() {
            if file == info.file_id {
                found_idx = Some((info, idx));
                break;
            }
        }

        let (file_info, file_idx) = found_idx.ok_or(Error::BadHandle)?;

        if file_info.dirty {
            match self.volume_type {
                VolumeType::Fat(ref mut fat) => {
                    debug!("Updating FAT info sector");
                    fat.update_info_sector(&self.block_device).await?;
                    debug!("Updating dir entry {:?}", file_info.entry);
                    if file_info.entry.size != 0 {
                        // If you have a length, you must have a cluster
                        assert!(file_info.entry.cluster.0 != 0);
                    }
                    let fat_type = fat.get_fat_type();
                    self.write_entry_to_disk(fat_type, &file_info.entry).await?;
                }
            };
        }

        self.open_files.swap_remove(file_idx);
        Ok(())
    }

    /// Check if any files or folders are open.
    pub fn has_open_handles(&self) -> bool {
        !(self.open_dirs.is_empty() || self.open_files.is_empty())
    }

    /// Consume self and return BlockDevice and TimeSource
    pub fn free(self) -> (D, T) {
        (self.block_device, self.time_source)
    }

    /// Check if a file is at End Of File.
    pub fn file_eof(&self, file: File) -> Result<bool, Error<D::Error>> {
        let file_idx = self.get_file_by_id(file)?;
        Ok(self.open_files[file_idx].eof())
    }

    /// Seek a file with an offset from the start of the file.
    pub fn file_seek_from_start(&mut self, file: File, offset: u32) -> Result<(), Error<D::Error>> {
        let file_idx = self.get_file_by_id(file)?;
        self.open_files[file_idx]
            .seek_from_start(offset)
            .map_err(|_| Error::InvalidOffset)?;
        Ok(())
    }

    /// Seek a file with an offset from the current position.
    pub fn file_seek_from_current(
        &mut self,
        file: File,
        offset: i32,
    ) -> Result<(), Error<D::Error>> {
        let file_idx = self.get_file_by_id(file)?;
        self.open_files[file_idx]
            .seek_from_current(offset)
            .map_err(|_| Error::InvalidOffset)?;
        Ok(())
    }

    /// Seek a file with an offset back from the end of the file.
    pub fn file_seek_from_end(&mut self, file: File, offset: u32) -> Result<(), Error<D::Error>> {
        let file_idx = self.get_file_by_id(file)?;
        self.open_files[file_idx]
            .seek_from_end(offset)
            .map_err(|_| Error::InvalidOffset)?;
        Ok(())
    }

    /// Get the length of a file
    pub fn file_length(&self, file: File) -> Result<u32, Error<D::Error>> {
        let file_idx = self.get_file_by_id(file)?;
        Ok(self.open_files[file_idx].length())
    }

    /// Get the current offset of a file
    pub fn file_offset(&self, file: File) -> Result<u32, Error<D::Error>> {
        let file_idx = self.get_file_by_id(file)?;
        Ok(self.open_files[file_idx].current_offset)
    }

    fn get_dir_by_id(&self, directory: Directory) -> Result<usize, Error<D::Error>> {
        for (idx, d) in self.open_dirs.iter().enumerate() {
            if d.directory_id == directory {
                return Ok(idx);
            }
        }
        Err(Error::BadHandle)
    }

    fn get_file_by_id(&self, file: File) -> Result<usize, Error<D::Error>> {
        for (idx, f) in self.open_files.iter().enumerate() {
            if f.file_id == file {
                return Ok(idx);
            }
        }
        Err(Error::BadHandle)
    }

    /// This function turns `desired_offset` into an appropriate block to be
    /// read. It either calculates this based on the start of the file, or
    /// from the last cluster we read - whichever is better.
    async fn find_data_on_disk(
        &self,
        start: &mut (u32, ClusterId),
        desired_offset: u32,
    ) -> Result<(BlockIdx, usize, usize), Error<D::Error>> {
        let bytes_per_cluster = match &self.volume_type {
            VolumeType::Fat(fat) => fat.bytes_per_cluster(),
        };
        // How many clusters forward do we need to go?
        let offset_from_cluster = desired_offset - start.0;
        let num_clusters = offset_from_cluster / bytes_per_cluster;
        let mut block_cache = BlockCache::empty();
        for _ in 0..num_clusters {
            start.1 = match &self.volume_type {
                VolumeType::Fat(fat) => {
                    fat.next_cluster(&self.block_device, start.1, &mut block_cache)
                        .await?
                }
            };
            start.0 += bytes_per_cluster;
        }
        // How many blocks in are we?
        let offset_from_cluster = desired_offset - start.0;
        assert!(offset_from_cluster < bytes_per_cluster);
        let num_blocks = BlockCount(offset_from_cluster / Block::LEN_U32);
        let block_idx = match &self.volume_type {
            VolumeType::Fat(fat) => fat.cluster_to_block(start.1),
        } + num_blocks;
        let block_offset = (desired_offset % Block::LEN_U32) as usize;
        let available = Block::LEN - block_offset;
        Ok((block_idx, block_offset, available))
    }

    /// Writes a Directory Entry to the disk
    async fn write_entry_to_disk(
        &self,
        fat_type: fat::FatType,
        entry: &DirEntry,
    ) -> Result<(), Error<D::Error>> {
        let mut blocks = [Block::new()];
        self.block_device
            .read(&mut blocks, entry.entry_block, "read")
            .await
            .map_err(Error::DeviceError)?;
        let block = &mut blocks[0];

        let start = usize::try_from(entry.entry_offset).map_err(|_| Error::ConversionError)?;
        block[start..start + 32].copy_from_slice(&entry.serialize(fat_type)[..]);

        self.block_device
            .write(&blocks, entry.entry_block)
            .await
            .map_err(Error::DeviceError)?;
        Ok(())
    }
}

/// Transform mode variants (ReadWriteCreate_Or_Append) to simple modes ReadWriteAppend or
/// ReadWriteCreate
fn solve_mode_variant(mode: Mode, dir_entry_is_some: bool) -> Mode {
    let mut mode = mode;
    if mode == Mode::ReadWriteCreateOrAppend {
        if dir_entry_is_some {
            mode = Mode::ReadWriteAppend;
        } else {
            mode = Mode::ReadWriteCreate;
        }
    } else if mode == Mode::ReadWriteCreateOrTruncate {
        if dir_entry_is_some {
            mode = Mode::ReadWriteTruncate;
        } else {
            mode = Mode::ReadWriteCreate;
        }
    }
    mode
}

// ****************************************************************************
//
// Unit Tests
//
// ****************************************************************************

// None
