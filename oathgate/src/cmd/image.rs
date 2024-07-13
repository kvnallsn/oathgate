//! Install and manage disk images

use std::{fs::File, path::PathBuf};

use clap::Subcommand;

use crate::{
    database::image::{DiskFormat, DiskImage},
    State,
};

#[derive(Debug, Subcommand)]
pub enum ImageCommand {
    /// Installs a disk image for use by virtual machines/shards
    Install {
        /// Path to disk image
        image: PathBuf,

        /// Name of this disk image
        #[clap(short, long)]
        name: Option<String>,
    },

    /// List installed/available disk images
    #[clap(alias = "ls")]
    List,
}

impl ImageCommand {
    /// Executes this kernel command
    ///
    /// ### Argument
    /// * `state` - Application state
    pub fn execute(self, state: &State) -> anyhow::Result<()> {
        match self {
            Self::Install { image, name } => image_install(state, image, name)?,
            Self::List => image_list(state)?,
        }

        Ok(())
    }
}

/// Installs a disk image into the oathgate system
///
/// This will copy the image to a pre-determined location on the filesystem and track the image in
/// the oathgate database
///
/// ### Arguments
/// * `state` - Application state
/// * `kernel` - Path to kernel to install on disk
/// * `version` - Kernel version (i.e., 6.9.0-32)
/// * `name` - Name to refer to this kernel by (or omitted to auto-generate)
fn image_install(state: &State, image: PathBuf, name: Option<String>) -> anyhow::Result<()> {
    let name = name.unwrap_or_else(|| state.generate_name());

    let mime = state.get_mime(&image)?;
    let format = match mime.as_str() {
        "application/x-qemu-disk" => DiskFormat::Qcow2,
        "application/octet-stream" => DiskFormat::Raw,
        mime => {
            println!("unknown mime type {mime}, setting format to raw");
            DiskFormat::Raw
        }
    };

    let bar = super::spinner(format!("hashing image '{name}'"));

    // hash the file to generate a unique id
    let mut src = File::options().write(false).read(true).open(image)?;
    let hash_id = super::hash_file(&mut src)?;

    bar.finish_with_message(format!("hashed image '{name}'"));
    let bar = super::spinner(format!("installing image '{name}'"));

    let image = DiskImage::new(state.ctx(), hash_id, &name, format);

    let mut dst = File::options()
        .write(true)
        .create(true)
        .append(false)
        .open(image.path(state))?;
    std::io::copy(&mut src, &mut dst)?;

    image.save(state.db())?;

    bar.finish_with_message(format!("installed image '{name}'"));
    Ok(())
}

/// Retrieves the list of disk images from the database
///
/// ### Arguments
/// * `state` - Application state
fn image_list(state: &State) -> anyhow::Result<()> {
    let images = DiskImage::get_all(state.db())?;
    super::draw_table(&images);
    Ok(())
}
