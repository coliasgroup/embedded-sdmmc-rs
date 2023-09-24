#![feature(async_fn_in_trait)]

//! Append File Example.
//!
//! ```bash
//! $ cargo run --example append_file -- ./disk.img
//! $ cargo run --example append_file -- /dev/mmcblk0
//! ```
//!
//! If you pass a block device it should be unmounted. No testing has been
//! performed with Windows raw block devices - please report back if you try
//! this! There is a gzipped example disk image which you can gunzip and test
//! with if you don't have a suitable block device.
//!
//! ```bash
//! zcat ./tests/disk.img.gz > ./disk.img
//! $ cargo run --example append_file -- ./disk.img
//! ```

extern crate embedded_sdmmc;

mod linux;
use linux::*;

const FILE_TO_APPEND: &str = "README.TXT";

use embedded_sdmmc::{Error, Mode, VolumeIdx, VolumeManager};

#[tokio::main]
async fn main() -> Result<(), embedded_sdmmc::Error<std::io::Error>> {
    env_logger::init();
    let mut args = std::env::args().skip(1);
    let filename = args.next().unwrap_or_else(|| "/dev/mmcblk0".into());
    let print_blocks = args.find(|x| x == "-v").map(|_| true).unwrap_or(false);
    let lbd = LinuxBlockDevice::new(filename, print_blocks).map_err(Error::DeviceError)?;
    let mut volume_mgr: VolumeManager<LinuxBlockDevice, Clock, 8, 8, 4> =
        VolumeManager::new_with_limits(lbd, Clock, 0xAA00_0000);
    let volume = volume_mgr.open_volume(VolumeIdx(0)).await?;
    let root_dir = volume_mgr.open_root_dir(volume)?;
    println!("\nCreating file {}...", FILE_TO_APPEND);
    let f = volume_mgr
        .open_file_in_dir(root_dir, FILE_TO_APPEND, Mode::ReadWriteAppend)
        .await?;
    volume_mgr
        .write(f, b"\r\n\r\nThis has been added to your file.\r\n")
        .await?;
    volume_mgr.close_file(f).await?;
    volume_mgr.close_dir(root_dir)?;
    Ok(())
}
