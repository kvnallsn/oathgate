//! Shard / virtual machines

use std::path::PathBuf;

use anyhow::{anyhow, Context};
use oathgate_net::types::MacAddress;
use oathgate_runner::config::{DiskConfig, KernelConfig, MachineConfig};
use rusqlite::{params, OptionalExtension, Row};
use uuid::Uuid;

use crate::{
    cmd::AsTable,
    process::{self, ProcessState},
    State,
};

use super::{image::DiskImage, kernel::Kernel, Database, Device};

/// A shard is a representation of a VM stored in the database
#[derive(Debug)]
pub struct Shard {
    /// Shard parameters
    params: ShardParams,

    /// Kernel image and configuration
    kernel: Kernel,

    /// Disk image to use / boot
    boot_disk: DiskImage,

    /// Networks associated with this shard
    networks: Vec<ShardNetwork>,
}

#[derive(Debug, Default)]
pub struct ShardBuilder {
    /// Name of this shard
    name: Option<String>,

    /// Type of CPU/processor of this shard
    cpu: Option<String>,

    /// Amount of RAM/memory, in megabytes
    memory: Option<u16>,

    /// Kernel to use for shard
    kernel: Option<Kernel>,

    /// Disk image to use for shard
    boot_disk: Option<DiskImage>,
}

#[derive(Debug)]
pub struct ShardParams {
    /// A unique id used internally, should not be exposed
    id: Uuid,

    /// Context id used to communicate using vhost-vsock devices
    cid: u32,

    /// Name of this shard
    name: String,

    /// Current state of the process (not saved in the database)
    state: ProcessState,

    /// Type of CPU/processor of this shard
    cpu: String,

    /// Amount of RAM/memory, in megabytes
    memory: u16,
}

#[derive(Debug)]
pub struct ShardNetwork {
    /// Unique id of the shard
    shard_id: Uuid,

    /// Unique id of the network
    network_id: Uuid,

    /// MAC address of the shard's interface
    mac: String,
}

impl Shard {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        let params = ShardParams::from_row(row)?;
        let kernel = Kernel::from_row(row)?;
        let boot_disk = DiskImage::from_row(row)?;
        let networks = Vec::new();

        Ok(Self {
            params,
            kernel,
            boot_disk,
            networks,
        })
    }

    /// Returns the shard with the specificed name, or None if one is not found
    ///
    /// ### Arguments
    /// * `db` - Reference to the database
    /// * `name` - Name of the shard
    pub fn get(db: &Database, name: &str) -> anyhow::Result<Option<Self>> {
        let shard = db.transaction(|conn| {
            let mut stmt = conn.prepare(
                "
                    SELECT
                        shards.id AS shard_id,
                        shards.name AS shard_name,
                        shards.pid AS shard_pid,
                        shards.cid AS shard_cid,
                        shards.cpu AS shard_cpu,
                        shards.memory AS shard_memory,
                        kernels.id AS kernel_id,
                        kernels.hash AS kernel_hash,
                        kernels.name AS kernel_name,
                        kernels.version AS kernel_version,
                        kernels.is_default AS kernel_default,
                        images.id AS image_id,
                        images.hash AS image_hash,
                        images.name AS image_name,
                        images.format AS image_format,
                        images.root AS image_root
                    FROM
                        shards
                    INNER JOIN kernels ON
                        kernels.id = shards.kernel
                    INNER JOIN images ON
                        images.id = shards.bootdisk
                    WHERE
                        shards.name = ?1
            ",
            )?;

            let shard = stmt.query_row(params![name], Self::from_row).optional()?;

            Ok(shard)
        })?;

        Ok(shard)
    }

    /// Returns all shards in the database
    ///
    /// ### Arguments
    /// * `db` - Reference to the database
    pub fn get_all(db: &Database) -> anyhow::Result<Vec<Shard>> {
        let shards = db.transaction(|conn| {
            let mut stmt = conn.prepare(
                "
                    SELECT
                        shards.id AS shard_id,
                        shards.name AS shard_name,
                        shards.pid AS shard_pid,
                        shards.cid AS shard_cid,
                        shards.cpu AS shard_cpu,
                        shards.memory AS shard_memory,
                        kernels.id AS kernel_id,
                        kernels.hash AS kernel_hash,
                        kernels.name AS kernel_name,
                        kernels.version AS kernel_version,
                        kernels.is_default AS kernel_default,
                        images.id AS image_id,
                        images.hash AS image_hash,
                        images.name AS image_name,
                        images.format AS image_format,
                        images.root AS image_root
                    FROM
                        shards
                    INNER JOIN kernels ON
                        kernels.id = shards.kernel
                    INNER JOIN images ON
                        images.id = shards.bootdisk
            ",
            )?;

            let shards = stmt
                .query_map(params![], Self::from_row)?
                .filter_map(|dev| dev.ok())
                .collect::<Vec<_>>();

            Ok(shards)
        })?;

        Ok(shards)
    }

    /// Inserts this record into the database
    pub fn save(&self, db: &Database) -> anyhow::Result<()> {
        db.transaction(|conn| {
            conn.execute(
                "INSERT INTO
                    shards (id, name, pid, cid, cpu, memory, kernel, bootdisk)
                 VALUES
                    (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name,
                    pid = excluded.pid,
                    cid = excluded.cid,
                    cpu = excluded.cpu,
                    memory = excluded.memory,
                    kernel = excluded.kernel,
                    bootdisk = excluded.bootdisk
                ",
                (
                    self.id(),
                    self.name(),
                    self.params.state.optional(),
                    self.cid(),
                    &self.params.cpu,
                    self.params.memory,
                    self.kernel.id,
                    self.boot_disk.id,
                ),
            )?;

            Ok(())
        })
        .context("unable to save state in database")?;

        Ok(())
    }

    /// Copies the necessary files to the running directory
    ///
    /// The location of the running directory is based on the application state (i.e., the base
    /// path specified when the application is started)
    pub fn deploy(&self, state: &State) -> anyhow::Result<()> {
        // create the directory to hold a shard's deployed files
        std::fs::create_dir_all(self.dir(state))?;

        // create a hard link for the kernel since it is opened read-only
        std::fs::hard_link(self.kernel.path(state), self.kernel_path(state))?;

        // copy the disk image so each instance gets it's own copy
        // QUESTION: is it worthwhile to use clones for qcow2 images?
        std::fs::copy(self.boot_disk.path(state), self.boot_disk_path(state))?;

        Ok(())
    }

    /// Associates a device with this shard
    ///
    /// ### Argumnets
    /// * `db` - Database connection
    /// * `dev` - Device to associate with shard
    pub fn add_device_ref(
        &self,
        db: &Database,
        dev: &Device,
        interface: &str,
    ) -> anyhow::Result<()> {
        db.transaction(|conn| {
            conn.execute(
                "INSERT INTO
                    shard_devices (device_id, shard_id, interface)
                VALUES
                    (?1, ?2, ?3)
                ON CONFLICT(device_id, shard_id) DO NOTHING",
                (dev.id(), self.id(), interface),
            )?;

            Ok(())
        })?;

        Ok(())
    }

    pub fn networks(&self, db: &Database) -> anyhow::Result<Vec<Device>> {
        let networks = db.transaction(|conn| {
            let mut stmt = conn.prepare(
                "
                SELECT
                    d.id, d.pid, d.name, d.device, d.config
                FROM devices AS d
                INNER JOIN shard_devices ON
                    shard_devices.device_id = d.id
                WHERE
                    shard_devices.shard_id = ?1",
            )?;

            let devices = stmt
                .query_map(params![self.id()], Device::from_row)?
                .filter_map(|dev| dev.ok())
                .collect::<Vec<_>>();

            Ok(devices)
        })?;

        Ok(networks)
    }

    /// Deletes this shard from the database
    ///
    /// ### Arguments
    /// * `db` - Reference to the database
    pub fn delete(&self, db: &Database) -> anyhow::Result<()> {
        db.transaction(|conn| {
            conn.execute("DELETE FROM shards WHERE id = ?1", (&self.id(),))?;
            Ok(())
        })?;

        Ok(())
    }

    /// Removes all shard files from disk and deletes the entry in the database
    ///
    /// ### Arguments
    /// * `state` - Appliation state
    pub fn purge(self, state: &State) -> anyhow::Result<()> {
        self.delete(state.db())?;

        let dir = self.dir(state);
        std::fs::remove_dir_all(dir)?;

        Ok(())
    }

    /// Returns this shard's unique identifier
    pub fn id(&self) -> Uuid {
        self.params.id
    }

    /// Returns the name of this shard
    pub fn name(&self) -> &str {
        self.params.name.as_str()
    }

    /// Returns the context id used with vhost-vsock devices
    pub fn cid(&self) -> u32 {
        self.params.cid
    }

    /// Returns true if this shard is currently running
    pub fn is_running(&self) -> bool {
        matches!(self.params.state, ProcessState::Running(_))
    }

    /// Updates the state of this process to running
    ///
    /// ### Arguments
    /// * `pid` - Process identifier of the newly started process
    pub fn set_running(&mut self, pid: i32) {
        self.params.state = ProcessState::Running(pid)
    }

    /// Updates the state of this process to stopped
    pub fn set_stopped(&mut self) {
        self.params.state = ProcessState::Stopped;
    }

    /// Stops a running shard
    pub fn stop(&mut self) -> anyhow::Result<()> {
        match self.params.state {
            ProcessState::Running(pid) => {
                process::stop(pid)?;
                self.set_stopped();
            }
            ProcessState::Dead(_) => self.set_stopped(),
            ProcessState::Stopped => (),
            ProcessState::PermissionDenied(_) => {
                return Err(anyhow!("unable to stop shard: permission denied"));
            }
        }

        Ok(())
    }

    /// Returns the path to the shard's directory (based on the base path)
    pub fn dir(&self, state: &State) -> PathBuf {
        state.shard_dir().join(self.name())
    }

    /// Returns the path to the kernel used to boot this shard
    pub fn kernel_path(&self, state: &State) -> PathBuf {
        self.dir(state).join(self.name()).with_extension("bin")
    }

    /// Returns the path to the disk image used to boot this shard
    pub fn boot_disk_path(&self, state: &State) -> PathBuf {
        self.dir(state).join(self.name()).with_extension("img")
    }

    /// Generates a `MachineConfig` to start this shard
    ///
    /// ### Arguments
    /// * `state` - Application State
    pub fn generate_machine_config(&self, state: &State) -> anyhow::Result<MachineConfig> {
        let kernel_path = self.kernel_path(state);
        let disk_path = self.boot_disk_path(state);
        let disk_format = self.boot_disk.format.to_string();
        let disk_root = self.boot_disk.root
            .map(|id| format!("/dev/vda{id}"))
            .unwrap_or_else(|| String::from("/dev/vda"));

        let kernel_cfg = KernelConfig::new(kernel_path, disk_root);
        let disk_cfg = DiskConfig::new(disk_path, disk_format);

        let memory = format!("{}m", self.params.memory);

        let cfg = MachineConfig::new(&self.params.cpu, memory, kernel_cfg, disk_cfg);

        Ok(cfg)
    }
}

impl ShardParams {
    /// Parses a Shard from a sqlite row
    ///
    /// ### Arguments
    /// * `row` - Row returned from database
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        let pid: Option<i32> = row.get("shard_pid")?;
        let state = match pid {
            Some(pid) => process::check(pid).unwrap(),
            None => ProcessState::Stopped,
        };

        Ok(Self {
            id: row.get("shard_id")?,
            name: row.get("shard_name")?,
            cid: row.get("shard_cid")?,
            cpu: row.get("shard_cpu")?,
            memory: row.get("shard_memory")?,
            state,
        })
    }
}

impl AsTable for Shard {
    fn header() -> &'static [&'static str] {
        &["Name", "State", "Context Id"]
    }

    fn update_col_width(&self, widths: &mut [usize]) {
        let cid = format!("0x{:02x}", self.cid());

        widths[0] = std::cmp::max(widths[0], self.name().len());
        widths[1] = std::cmp::max(widths[1], self.params.state.to_string().len());
        widths[2] = std::cmp::max(widths[2], cid.to_string().len());
    }

    fn as_table_row(&self, widths: &[usize]) {
        let cid = format!("0x{:02x}", self.cid());

        self.print_field(self.name(), widths[0]);
        self.print_field(&self.params.state.styled(), widths[1]);
        self.print_field(&cid, widths[2]);
    }
}

impl ShardBuilder {
    /// Sets the shard's name
    ///
    /// ### Arguments
    /// * `name` - New name for shard
    pub fn name<S: Into<String>>(&mut self, name: S) -> &mut Self {
        self.name = Some(name.into());
        self
    }

    /// Sets the CPU to emulate
    ///
    /// ### Arguments
    /// * `cpu` - CPU model (e.g. q35) to emulate
    pub fn cpu<S: Into<String>>(&mut self, cpu: S) -> &mut Self {
        self.cpu = Some(cpu.into());
        self
    }

    /// Sets the amount of memory (RAM)
    ///
    /// ### Arguments
    /// * `mem` - Amount of memory to allocate, in megabytes (MB)
    pub fn memory(&mut self, mem: u16) -> &mut Self {
        self.memory = Some(mem);
        self
    }

    /// Sets the kernel to use/boot
    ///
    /// ### Arguments
    /// * `kernel` - Linux kernel image
    pub fn kernel(&mut self, kernel: Kernel) -> &mut Self {
        self.kernel = Some(kernel);
        self
    }

    /// Sets the drive/disk to boot
    ///
    /// ### Arguments
    /// * `disk` - DiskImage from which to boot shard
    pub fn boot_disk(&mut self, disk: DiskImage) -> &mut Self {
        self.boot_disk = Some(disk);
        self
    }

    /// Adds a network association to this shard
    ///
    /// ### Arguments
    /// * `net` - Network to connect
    /// * `mac` - MAC address of network interface
    pub fn add_network(&mut self, net: Device, mac: MacAddress) -> &mut Self {
        self
    }

    /// Build the shard and configuration to store in database
    pub fn build(self, state: &State) -> anyhow::Result<Shard> {
        // insert shard params
        let params = ShardParams {
            id: state.generate_id(),
            cid: state.generate_cid(),
            name: self.name.ok_or_else(|| anyhow!("name field is required"))?,
            state: ProcessState::Stopped,
            cpu: self.cpu.ok_or_else(|| anyhow!("cpu type is required"))?,
            memory: self
                .memory
                .ok_or_else(|| anyhow!("memory field is required"))?,
        };

        let shard = Shard {
            params,
            kernel: self
                .kernel
                .ok_or_else(|| anyhow!("kernel field is required"))?,
            boot_disk: self
                .boot_disk
                .ok_or_else(|| anyhow!("boot disk field is required"))?,
            networks: Vec::new(),
        };

        Ok(shard)
    }
}
