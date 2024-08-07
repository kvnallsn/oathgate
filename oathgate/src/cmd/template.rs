//! Shard Templates

use std::{
    fs::{metadata, File},
    io::{Read, Seek},
    path::PathBuf,
};

use anyhow::{anyhow, Context};
use clap::Subcommand;
use flate2::read::GzDecoder;
use oathgate_runner::config::MachineConfig;
use tar::Archive;
use zip::ZipArchive;

use crate::{
    database::{
        shard::{Shard, ShardTemplate},
        Device,
    },
    State,
};

use super::draw_table;

const GZIP_MAGIC_HEADER: [u8; 2] = [0x1F, 0x8B];

#[derive(Debug, Subcommand)]
pub enum TemplateCommand {
    /// Imports a shard archive into the system
    Import {
        /// Path to compressed vm image
        shard: PathBuf,

        /// Name of this shard template
        #[clap(short, long)]
        name: Option<String>,
    },

    /// Deploys a template as a running shard
    Deploy {
        /// List of networks that will be attached to this shard
        #[clap(short, long)]
        networks: Vec<String>,

        /// Name of template to deploy
        template: String,

        /// Name of shard (or omit to auto-generate)
        name: Option<String>,
    },

    /// Lists all installed templates
    #[clap(alias = "status", alias = "ls")]
    List,
}

impl TemplateCommand {
    /// Executes a template command
    pub fn execute(self, state: &State) -> anyhow::Result<()> {
        match self {
            Self::Import { shard, name } => import_template_zip(state, shard, name)?,
            Self::Deploy {
                networks,
                template,
                name,
            } => deploy_template(state, template, name, networks)?,
            Self::List => list_templates(state)?,
        }

        Ok(())
    }
}

/// Copies a shard's packed vm archive into the shard directory
fn import_template(state: &State, archive: PathBuf, name: Option<String>) -> anyhow::Result<()> {
    let name = name.unwrap_or_else(|| state.generate_name());
    let bar = super::spinner(format!("importing template {name}"));

    if !archive.exists() {
        return Err(anyhow!(
            "archive file ({}) does not exist",
            archive.display()
        ));
    }

    if !archive.is_file() {
        return Err(anyhow!(
            "archive file ({}) exists, but is not a file",
            archive.display()
        ));
    }

    let mut hdr = [0u8; 2];
    let mut src = File::open(&archive)?;
    src.read_exact(&mut hdr)?;

    if hdr != GZIP_MAGIC_HEADER {
        return Err(anyhow!(
            "archive src ({}) is not gzip'd tar archive",
            archive.display()
        ));
    }

    src.seek(std::io::SeekFrom::Start(0))?;

    let mut archive = Archive::new(GzDecoder::new(src));

    for entry in archive.entries()? {
        let entry = entry?;
        println!("file: {:?}", entry.header().path()?);
    }

    /*
    let mut dst = File::options()
        .write(true)
        .create(true)
        .open(state.archive_dir().join(&name).with_extension("tgz"))?;

    std::io::copy(&mut src, &mut dst)?;
    */

    //let template = ShardTemplate::new(state.ctx(), name);
    //template.save(state.db())?;

    bar.finish_with_message("import complete");

    Ok(())
}

/// Copies a shard's packed vm archive into the shard directory
fn import_template_zip(state: &State, archive: PathBuf, name: Option<String>) -> anyhow::Result<()> {
    let name = name.unwrap_or_else(|| state.generate_name());
    let bar = super::spinner(format!("importing template {name}"));

    if !archive.exists() {
        return Err(anyhow!(
            "archive file ({}) does not exist",
            archive.display()
        ));
    }

    if !archive.is_file() {
        return Err(anyhow!(
            "archive file ({}) exists, but is not a file",
            archive.display()
        ));
    }

    let mut archive = ZipArchive::new(File::open(archive)?)?;

    let metadata = archive.by_name("METADATA")?;
    let mc = MachineConfig::read_yaml(metadata)?;
    let template = ShardTemplate::from_machine(state.ctx(), &name, mc);
    template.save(state.db())?;

    bar.finish_with_message(format!("import complete. template name is '{name}'"));
    Ok(())
}

/// Deploys a shard template
fn deploy_template(
    state: &State,
    template: String,
    name: Option<String>,
    networks: Vec<String>,
) -> anyhow::Result<()> {
    let name = name.unwrap_or_else(|| state.generate_name());

    /*
    // validate all networks/bridges
    let mut missing = Vec::new();
    let networks = networks
        .into_iter()
        .flat_map(|net| match Device::get(state.db(), &net).ok().flatten() {
            Some(dev) => Some(dev),
            None => {
                missing.push(net);
                None
            }
        })
        .collect::<Vec<_>>();

    if !missing.is_empty() {
        return Err(anyhow!("unable to find networks: {}", missing.join(", ")));
    }
    */

    let bar = super::spinner(format!("deploying shard template '{template}' as '{name}'"));

    let template = ShardTemplate::get(state.db(), &template)?;

    let archive = File::open(
        state
            .archive_dir()
            .join(&template.name())
            .with_extension("tgz"),
    )
    .context("archive file not found")?;

    let mut archive = Archive::new(GzDecoder::new(archive));

    let dest = state.shard_dir().join(&name);
    std::fs::create_dir_all(&dest)?;

    archive.unpack(&dest)?;

    let shard = Shard::new(state.ctx(), &name);

    shard.save(&state.db())?;

    /*
    for net in networks {
        shard.add_device_ref(state.db(), &net, "")?;
    }
    */

    bar.finish_with_message("deployment complete");

    Ok(())
}

/// Lists all available templates
fn list_templates(state: &State) -> anyhow::Result<()> {
    let templates = ShardTemplate::get_all(state.db())?;
    match templates.is_empty() {
        true => println!("no templates found"),
        false => draw_table(&templates),
    }

    Ok(())
}
