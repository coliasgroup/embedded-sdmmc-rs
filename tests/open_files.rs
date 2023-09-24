#![feature(async_fn_in_trait)]

//! File opening related tests

use embedded_sdmmc::{Error, Mode, VolumeIdx, VolumeManager};

mod utils;

#[tokio::test]
async fn open_files() {
    let time_source = utils::make_time_source();
    let disk = utils::make_block_device(utils::DISK_SOURCE).unwrap();
    let mut volume_mgr: VolumeManager<utils::RamDisk<Vec<u8>>, utils::TestTimeSource, 4, 2, 1> =
        VolumeManager::new_with_limits(disk, time_source, 0xAA00_0000);
    let volume = volume_mgr
        .open_volume(VolumeIdx(0))
        .await
        .expect("open volume");
    let root_dir = volume_mgr.open_root_dir(volume).expect("open root dir");

    // Open with string
    let f = volume_mgr
        .open_file_in_dir(root_dir, "README.TXT", Mode::ReadWriteTruncate)
        .await
        .expect("open file");

    let r = volume_mgr
        .open_file_in_dir(root_dir, "README.TXT", Mode::ReadOnly)
        .await;
    let Err(Error::FileAlreadyOpen) = r else {
        panic!("Expected to not open file twice: {r:?}");
    };

    volume_mgr.close_file(f).await.expect("close file");

    // Open with SFN

    let dir_entry = volume_mgr
        .find_directory_entry(root_dir, "README.TXT")
        .await
        .expect("find file");

    let f = volume_mgr
        .open_file_in_dir(root_dir, &dir_entry.name, Mode::ReadWriteCreateOrAppend)
        .await
        .expect("open file with dir entry");

    let r = volume_mgr
        .open_file_in_dir(root_dir, &dir_entry.name, Mode::ReadOnly)
        .await;
    let Err(Error::FileAlreadyOpen) = r else {
        panic!("Expected to not open file twice: {r:?}");
    };

    // Can still spot duplicates even if name given the other way around
    let r = volume_mgr
        .open_file_in_dir(root_dir, "README.TXT", Mode::ReadOnly)
        .await;
    let Err(Error::FileAlreadyOpen) = r else {
        panic!("Expected to not open file twice: {r:?}");
    };

    let f2 = volume_mgr
        .open_file_in_dir(root_dir, "64MB.DAT", Mode::ReadWriteTruncate)
        .await
        .expect("open file");

    // Hit file limit
    let r = volume_mgr
        .open_file_in_dir(root_dir, "EMPTY.DAT", Mode::ReadOnly)
        .await;
    let Err(Error::TooManyOpenFiles) = r else {
        panic!("Expected to run out of file handles: {r:?}");
    };

    volume_mgr.close_file(f).await.expect("close file");
    volume_mgr.close_file(f2).await.expect("close file");

    // File not found
    let r = volume_mgr
        .open_file_in_dir(root_dir, "README.TXS", Mode::ReadOnly)
        .await;
    let Err(Error::FileNotFound) = r else {
        panic!("Expected to not open missing file: {r:?}");
    };

    // Create a new file
    let f3 = volume_mgr
        .open_file_in_dir(root_dir, "NEWFILE.DAT", Mode::ReadWriteCreate)
        .await
        .expect("open file");

    volume_mgr.write(f3, b"12345").await.expect("write to file");
    volume_mgr.write(f3, b"67890").await.expect("write to file");
    volume_mgr.close_file(f3).await.expect("close file");

    // Open our new file
    let f3 = volume_mgr
        .open_file_in_dir(root_dir, "NEWFILE.DAT", Mode::ReadOnly)
        .await
        .expect("open file");
    // Should have 10 bytes in it
    assert_eq!(volume_mgr.file_length(f3).expect("file length"), 10);
    volume_mgr.close_file(f3).await.expect("close file");

    volume_mgr.close_dir(root_dir).expect("close dir");
    volume_mgr.close_volume(volume).expect("close volume");
}
