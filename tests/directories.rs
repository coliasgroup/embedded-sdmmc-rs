#![feature(async_fn_in_trait)]

//! Directory related tests

use embedded_sdmmc::ShortFileName;

mod utils;

#[derive(Debug, Clone)]
struct ExpectedDirEntry {
    name: String,
    mtime: String,
    ctime: String,
    size: u32,
    is_dir: bool,
}

impl PartialEq<embedded_sdmmc::DirEntry> for ExpectedDirEntry {
    fn eq(&self, other: &embedded_sdmmc::DirEntry) -> bool {
        if other.name.to_string() != self.name {
            return false;
        }
        if format!("{}", other.mtime) != self.mtime {
            return false;
        }
        if format!("{}", other.ctime) != self.ctime {
            return false;
        }
        if other.size != self.size {
            return false;
        }
        if other.attributes.is_directory() != self.is_dir {
            return false;
        }
        true
    }
}

#[tokio::test]
async fn fat16_root_directory_listing() {
    let time_source = utils::make_time_source();
    let disk = utils::make_block_device(utils::DISK_SOURCE).unwrap();
    let mut volume_mgr = embedded_sdmmc::VolumeManager::new(disk, time_source);

    let fat16_volume = volume_mgr
        .open_volume(embedded_sdmmc::VolumeIdx(0))
        .await
        .expect("open volume 0");
    let root_dir = volume_mgr
        .open_root_dir(fat16_volume)
        .expect("open root dir");

    let expected = [
        ExpectedDirEntry {
            name: String::from("README.TXT"),
            mtime: String::from("2018-12-09 19:22:34"),
            ctime: String::from("2018-12-09 19:22:34"),
            size: 258,
            is_dir: false,
        },
        ExpectedDirEntry {
            name: String::from("EMPTY.DAT"),
            mtime: String::from("2018-12-09 19:21:16"),
            ctime: String::from("2018-12-09 19:21:16"),
            size: 0,
            is_dir: false,
        },
        ExpectedDirEntry {
            name: String::from("TEST"),
            mtime: String::from("2018-12-09 19:23:16"),
            ctime: String::from("2018-12-09 19:23:16"),
            size: 0,
            is_dir: true,
        },
        ExpectedDirEntry {
            name: String::from("64MB.DAT"),
            mtime: String::from("2018-12-09 19:21:38"),
            ctime: String::from("2018-12-09 19:21:38"),
            size: 64 * 1024 * 1024,
            is_dir: false,
        },
    ];

    let mut listing = Vec::new();

    volume_mgr
        .iterate_dir(root_dir, |d| {
            listing.push(d.clone());
        })
        .await
        .expect("iterate directory");

    assert_eq!(expected.len(), listing.len());
    for (expected_entry, given_entry) in expected.iter().zip(listing.iter()) {
        assert_eq!(
            expected_entry, given_entry,
            "{:#?} does not match {:#?}",
            given_entry, expected_entry
        );
    }
}

#[tokio::test]
async fn fat16_sub_directory_listing() {
    let time_source = utils::make_time_source();
    let disk = utils::make_block_device(utils::DISK_SOURCE).unwrap();
    let mut volume_mgr = embedded_sdmmc::VolumeManager::new(disk, time_source);

    let fat16_volume = volume_mgr
        .open_volume(embedded_sdmmc::VolumeIdx(0))
        .await
        .expect("open volume 0");
    let root_dir = volume_mgr
        .open_root_dir(fat16_volume)
        .expect("open root dir");
    let test_dir = volume_mgr
        .open_dir(root_dir, "TEST")
        .await
        .expect("open test dir");

    let expected = [
        ExpectedDirEntry {
            name: String::from("."),
            mtime: String::from("2018-12-09 19:21:02"),
            ctime: String::from("2018-12-09 19:21:02"),
            size: 0,
            is_dir: true,
        },
        ExpectedDirEntry {
            name: String::from(".."),
            mtime: String::from("2018-12-09 19:21:02"),
            ctime: String::from("2018-12-09 19:21:02"),
            size: 0,
            is_dir: true,
        },
        ExpectedDirEntry {
            name: String::from("TEST.DAT"),
            mtime: String::from("2018-12-09 19:22:12"),
            ctime: String::from("2018-12-09 19:22:12"),
            size: 3500,
            is_dir: false,
        },
    ];

    let mut listing = Vec::new();
    let mut count = 0;

    volume_mgr
        .iterate_dir(test_dir, |d| {
            if count == 0 {
                assert!(d.name == ShortFileName::this_dir());
            } else if count == 1 {
                assert!(d.name == ShortFileName::parent_dir());
            }
            count += 1;
            listing.push(d.clone());
        })
        .await
        .expect("iterate directory");

    assert_eq!(expected.len(), listing.len());
    for (expected_entry, given_entry) in expected.iter().zip(listing.iter()) {
        assert_eq!(
            expected_entry, given_entry,
            "{:#?} does not match {:#?}",
            given_entry, expected_entry
        );
    }
}

#[tokio::test]
async fn fat32_root_directory_listing() {
    let time_source = utils::make_time_source();
    let disk = utils::make_block_device(utils::DISK_SOURCE).unwrap();
    let mut volume_mgr = embedded_sdmmc::VolumeManager::new(disk, time_source);

    let fat32_volume = volume_mgr
        .open_volume(embedded_sdmmc::VolumeIdx(1))
        .await
        .expect("open volume 1");
    let root_dir = volume_mgr
        .open_root_dir(fat32_volume)
        .expect("open root dir");

    let expected = [
        ExpectedDirEntry {
            name: String::from("64MB.DAT"),
            mtime: String::from("2018-12-09 19:22:56"),
            ctime: String::from("2018-12-09 19:22:56"),
            size: 64 * 1024 * 1024,
            is_dir: false,
        },
        ExpectedDirEntry {
            name: String::from("EMPTY.DAT"),
            mtime: String::from("2018-12-09 19:22:56"),
            ctime: String::from("2018-12-09 19:22:56"),
            size: 0,
            is_dir: false,
        },
        ExpectedDirEntry {
            name: String::from("README.TXT"),
            mtime: String::from("2023-09-21 09:48:06"),
            ctime: String::from("2018-12-09 19:22:56"),
            size: 258,
            is_dir: false,
        },
        ExpectedDirEntry {
            name: String::from("TEST"),
            mtime: String::from("2018-12-09 19:23:20"),
            ctime: String::from("2018-12-09 19:23:20"),
            size: 0,
            is_dir: true,
        },
    ];

    let mut listing = Vec::new();

    volume_mgr
        .iterate_dir(root_dir, |d| {
            listing.push(d.clone());
        })
        .await
        .expect("iterate directory");

    assert_eq!(expected.len(), listing.len());
    for (expected_entry, given_entry) in expected.iter().zip(listing.iter()) {
        assert_eq!(
            expected_entry, given_entry,
            "{:#?} does not match {:#?}",
            given_entry, expected_entry
        );
    }
}

#[tokio::test]
async fn open_dir_twice() {
    let time_source = utils::make_time_source();
    let disk = utils::make_block_device(utils::DISK_SOURCE).unwrap();
    let mut volume_mgr = embedded_sdmmc::VolumeManager::new(disk, time_source);

    let fat32_volume = volume_mgr
        .open_volume(embedded_sdmmc::VolumeIdx(1))
        .await
        .expect("open volume 1");

    let root_dir = volume_mgr
        .open_root_dir(fat32_volume)
        .expect("open root dir");

    let r = volume_mgr.open_root_dir(fat32_volume);
    let Err(embedded_sdmmc::Error::DirAlreadyOpen) = r else {
        panic!("Expected to fail opening the root dir twice: {r:?}");
    };

    let r = volume_mgr.open_dir(root_dir, "README.TXT").await;
    let Err(embedded_sdmmc::Error::OpenedFileAsDir) = r else {
        panic!("Expected to fail opening file as dir: {r:?}");
    };

    let test_dir = volume_mgr
        .open_dir(root_dir, "TEST")
        .await
        .expect("open test dir");

    let r = volume_mgr.open_dir(root_dir, "TEST").await;
    let Err(embedded_sdmmc::Error::DirAlreadyOpen) = r else {
        panic!("Expected to fail opening the dir twice: {r:?}");
    };

    volume_mgr.close_dir(root_dir).expect("close root dir");
    volume_mgr.close_dir(test_dir).expect("close test dir");

    let r = volume_mgr.close_dir(test_dir);
    let Err(embedded_sdmmc::Error::BadHandle) = r else {
        panic!("Expected to fail closing the dir twice: {r:?}");
    };
}

#[tokio::test]
async fn open_too_many_dirs() {
    let time_source = utils::make_time_source();
    let disk = utils::make_block_device(utils::DISK_SOURCE).unwrap();
    let mut volume_mgr: embedded_sdmmc::VolumeManager<
        utils::RamDisk<Vec<u8>>,
        utils::TestTimeSource,
        1,
        4,
        2,
    > = embedded_sdmmc::VolumeManager::new_with_limits(disk, time_source, 0x1000_0000);

    let fat32_volume = volume_mgr
        .open_volume(embedded_sdmmc::VolumeIdx(1))
        .await
        .expect("open volume 1");
    let root_dir = volume_mgr
        .open_root_dir(fat32_volume)
        .expect("open root dir");

    let Err(embedded_sdmmc::Error::TooManyOpenDirs) = volume_mgr.open_dir(root_dir, "TEST").await
    else {
        panic!("Expected to fail at opening too many dirs");
    };
}

#[tokio::test]
async fn find_dir_entry() {
    let time_source = utils::make_time_source();
    let disk = utils::make_block_device(utils::DISK_SOURCE).unwrap();
    let mut volume_mgr = embedded_sdmmc::VolumeManager::new(disk, time_source);

    let fat32_volume = volume_mgr
        .open_volume(embedded_sdmmc::VolumeIdx(1))
        .await
        .expect("open volume 1");

    let root_dir = volume_mgr
        .open_root_dir(fat32_volume)
        .expect("open root dir");

    let dir_entry = volume_mgr
        .find_directory_entry(root_dir, "README.TXT")
        .await
        .expect("Find directory entry");
    assert!(dir_entry.attributes.is_archive());
    assert!(!dir_entry.attributes.is_directory());
    assert!(!dir_entry.attributes.is_hidden());
    assert!(!dir_entry.attributes.is_lfn());
    assert!(!dir_entry.attributes.is_system());
    assert!(!dir_entry.attributes.is_volume());

    let r = volume_mgr
        .find_directory_entry(root_dir, "README.TXS")
        .await;
    let Err(embedded_sdmmc::Error::FileNotFound) = r else {
        panic!("Expected not to find file: {r:?}");
    };
}

// ****************************************************************************
//
// End Of File
//
// ****************************************************************************
