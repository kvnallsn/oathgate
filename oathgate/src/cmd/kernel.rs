//! Manage and install linux kernels

use std::{fs::File, io::Seek, path::{Path, PathBuf}};

use clap::Subcommand;

use crate::{database::kernel::Kernel, State};

#[derive(Debug, Subcommand)]
pub enum KernelCommand {
    /// Installs a kernel for use by virtual machines/shards
    Install {
        /// Path to kernel image
        kernel: PathBuf,

        /// Name of this kernel image
        #[clap(short, long)]
        name: Option<String>,
    },

    /// List installed/available kernels
    #[clap(alias = "ls")]
    List,
}

impl KernelCommand {
    /// Executes this kernel command
    ///
    /// ### Argument
    /// * `state` - Application state
    pub fn execute(self, state: &State) -> anyhow::Result<()> {
        match self {
            Self::Install {
                kernel,
                name,
            } => kernel_install(state, kernel, name)?,
            Self::List => kernel_list(state)?,
        }

        Ok(())
    }
}

/// Installs a kernel into the oathgate system
///
/// This will copy the kernel to a pre-determined location on the filesystem and create an entry in
/// the database to track the kernel.
///
/// ### Arguments
/// * `state` - Application state
/// * `kernel` - Path to kernel to install on disk
/// * `version` - Kernel version (i.e., 6.9.0-32)
/// * `name` - Name to refer to this kernel by (or omitted to auto-generate)
pub fn kernel_install(
    state: &State,
    kernel: PathBuf,
    name: Option<String>,
) -> anyhow::Result<()> {
    let name = name.unwrap_or_else(|| state.generate_name());

    // attempt to detect version
    let version = match detect_kernel_version(&kernel)? {
        Some(vers) => vers,
        None => {
            println!(
                "Unable to detect kernel version. Are you sure {} is a linux kernel image? If so, specify the version",
                kernel.display()
            );

            dialoguer::Input::new().with_prompt("Kernel version").interact_text()?
        }
    };

    let bar = super::spinner(format!("installing kernel '{name}' (version {version})"));

    // hash the file to generate a unique id
    let mut src = File::options().write(false).read(true).open(kernel)?;
    let hash_id = super::hash_file(&mut src)?;
    src.seek(std::io::SeekFrom::Start(0))?;

    let kernel = Kernel::new(state.ctx(), hash_id, &name, &version);

    let dst = state.kernel_dir().join(kernel.id_str()).with_extension("bin");
    let mut dst = File::options().write(true).create(true).append(false).open(dst)?;
    std::io::copy(&mut src, &mut dst)?;

    kernel.save(state.db())?;

    bar.finish_with_message(format!("installed kernel '{name}' (version {version})"));

    Ok(())
}

/// Retrieves the list of kernels from the database
///
/// ### Arguments
/// * `state` - Application state
pub fn kernel_list(state: &State) -> anyhow::Result<()> {
    let kernels = Kernel::get_all(state.db())?;

    super::draw_table(&kernels);

    Ok(())
}

/// Attempts to detect the kernel version from a file
///
/// Methods (tried in order)
/// 1. Check output of file command
///
/// ### Arguments
/// * `path` - Path to kernel image
fn detect_kernel_version<P: AsRef<Path>>(path: P) -> anyhow::Result<Option<String>> {
    let path = path.as_ref();

    let extract = |output: &str| {
        let iter = output.split(" ").into_iter();
        iter.skip_while(|s| *s != "version").skip(1).next().map(|s| s.to_string())
    };

    let vers = std::process::Command::new("file").arg("-b").arg(path).output()?;
    let output = String::from_utf8(vers.stdout)?;
    if output.starts_with("Linux kernel") {
        return Ok(extract(&output));
    }

    if output.starts_with("ELF") {
        // attempt to run strings
        let output = std::process::Command::new("strings").arg(path).output()?;
        let iter = String::from_utf8(output.stdout)?;
        let vers = iter.lines().filter(|s| s.starts_with("Linux version")).next();
        if let Some(vers) = vers {
            return Ok(extract(vers));
        }
    }

    Ok(None)
}
