//! A database logger

use std::{
    borrow::Cow,
    collections::BTreeMap,
    fmt::Display,
    path::Path,
    str::FromStr,
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::anyhow;
use clap::ValueEnum;
use console::Style;
use serde::{Deserialize, Serialize};
use time::{format_description::well_known::Rfc2822, OffsetDateTime};
use tracing::{span, Level, Metadata};
use uuid::Uuid;

use crate::database::{log::LogEntry, Database};

pub struct SqliteSubscriber {
    db: Database,
    max_level: Level,
    next_id: AtomicU64,
    device_id: Uuid,
}

pub struct SqliteSubscriberBuilder {
    max_level: Level,
    device_id: Option<Uuid>,
}

#[derive(Clone, Debug, Serialize, ValueEnum)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

#[derive(Debug, Serialize)]
pub struct OathgateEvent<'a> {
    pub target: Cow<'a, str>,
    pub line: Option<u32>,
    pub module: Option<Cow<'a, str>>,
    pub level: LogLevel,
    pub data: BTreeMap<Cow<'a, str>, DataType<'a>>,
    pub ts: OffsetDateTime,
}

#[derive(Debug, Deserialize, Serialize)]
pub enum DataType<'a> {
    Float(f64),
    U64(u64),
    I64(i64),
    U128(u128),
    I128(i128),
    Boolean(bool),
    Str(Cow<'a, str>),
}

impl SqliteSubscriberBuilder {
    /// Sets the max level to record
    pub fn with_max_level(mut self, level: Level) -> Self {
        self.max_level = level;
        self
    }

    /// Sets the device id associated with this logger
    pub fn with_device_id(mut self, id: Uuid) -> Self {
        self.device_id = Some(id);
        self
    }

    /// Finishes building the subscriber and installs it as the global default
    pub fn finish<P: AsRef<Path>>(self, path: P) -> anyhow::Result<SqliteSubscriber> {
        let db = Database::open(path)?;
        let device_id = self.device_id.ok_or_else(|| anyhow!("missing device id"))?;

        let subscriber = SqliteSubscriber {
            db,
            max_level: self.max_level,
            next_id: AtomicU64::new(0),
            device_id
        };


        //tracing::subscriber::set_global_default(subscriber)?;
        Ok(subscriber)
    }
}

impl SqliteSubscriber {
    /// Returns a new builder for our subscriber
    pub fn builder() -> SqliteSubscriberBuilder {
        SqliteSubscriberBuilder {
            max_level: Level::ERROR,
            device_id: None,
        }
    }
}

impl tracing::Subscriber for SqliteSubscriber {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        let level = metadata.level();
        level <= &self.max_level
    }

    fn new_span(&self, _span: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(self.next_id.fetch_add(1, Ordering::Relaxed))
    }

    fn enter(&self, _span: &span::Id) {
        // TODO
    }

    fn exit(&self, _span: &span::Id) {
        // TODO
    }

    fn event(&self, event: &tracing::Event<'_>) {
        let mut visitor = OathgateEvent::new(event.metadata());
        event.record(&mut visitor);

        if let Err(error) = LogEntry::save(&self.db, Some(self.device_id), &visitor) {
            eprintln!("sqlite logger: {error}");
        }
    }

    fn record_follows_from(&self, _span: &span::Id, _follows: &span::Id) {
        // TODO
    }

    fn record(&self, _span: &span::Id, _values: &span::Record<'_>) {
        // TODO
    }
}

impl<'a> tracing::field::Visit for OathgateEvent<'a> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let dbg = format!("{value:?}");

        match field.name() {
            name if name.starts_with("r#") => {
                self.data.insert(name[2..].into(), DataType::from(dbg))
            }
            name => self.data.insert(name.into(), DataType::from(dbg)),
        };
    }

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        self.data.insert(field.name().into(), DataType::from(value));
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.data.insert(field.name().into(), DataType::from(value));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.data.insert(field.name().into(), DataType::from(value));
    }

    fn record_i128(&mut self, field: &tracing::field::Field, value: i128) {
        self.data.insert(field.name().into(), DataType::from(value));
    }

    fn record_u128(&mut self, field: &tracing::field::Field, value: u128) {
        self.data.insert(field.name().into(), DataType::from(value));
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.data.insert(field.name().into(), DataType::from(value));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.data
            .insert(field.name().into(), DataType::from(String::from(value)));
    }

    fn record_error(
        &mut self,
        field: &tracing::field::Field,
        value: &(dyn std::error::Error + 'static),
    ) {
        let err = format!("{value}");
        self.data.insert(field.name().into(), DataType::from(err));
    }
}

impl<'a> OathgateEvent<'a> {
    pub fn new(metadata: &'a Metadata<'_>) -> Self {
        let data = BTreeMap::new();
        let ts = OffsetDateTime::now_utc();

        Self {
            target: metadata.target().into(),
            line: metadata.line(),
            module: metadata.module_path().map(|m| Cow::Borrowed(m)),
            level: LogLevel::from(*metadata.level()),
            data,
            ts,
        }
    }

    pub fn display(&self) {
        let style = self.level.style();
        let style_dim = self.level.style().dim();
        let dim = Style::new().dim();

        let pipe = format!("{}", style_dim.apply_to("\u{251c}"));
        let arrow = format!("{}", style_dim.apply_to("\u{2514}"));

        match self.data.get("message") {
            Some(msg) => println!("{}", style.apply_to(msg)),
            None => println!("{}", style.apply_to("<no log message>")),
        }

        for (k, v) in &self.data {
            if *k == "message" {
                continue;
            }

            println!("{pipe} {} {} {v}", dim.apply_to(k), dim.apply_to("="));
        }

        println!(
            "{pipe} {} {}",
            dim.apply_to("level ="),
            style_dim.apply_to(&self.level)
        );

        println!("{pipe} {} {}", dim.apply_to("target ="), self.target);

        println!(
            "{arrow} {} {}",
            dim.apply_to("datetime ="),
            self.ts.format(&Rfc2822).unwrap()
        );

        println!(""); // blank line for spacing
    }
}

impl<'a> From<bool> for DataType<'a> {
    fn from(value: bool) -> Self {
        DataType::Boolean(value)
    }
}

impl<'a> From<f64> for DataType<'a> {
    fn from(value: f64) -> Self {
        DataType::Float(value)
    }
}

impl<'a> From<u64> for DataType<'a> {
    fn from(value: u64) -> Self {
        DataType::U64(value)
    }
}

impl<'a> From<i64> for DataType<'a> {
    fn from(value: i64) -> Self {
        DataType::I64(value)
    }
}

impl<'a> From<u128> for DataType<'a> {
    fn from(value: u128) -> Self {
        DataType::U128(value)
    }
}

impl<'a> From<i128> for DataType<'a> {
    fn from(value: i128) -> Self {
        DataType::I128(value)
    }
}

impl<'a> From<&'a str> for DataType<'a> {
    fn from(value: &'a str) -> Self {
        DataType::Str(Cow::Borrowed(value))
    }
}

impl<'a> From<String> for DataType<'a> {
    fn from(value: String) -> Self {
        DataType::Str(Cow::Owned(value))
    }
}

impl<'a> Display for DataType<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Str(str) => write!(f, "{str}"),
            Self::Float(fl) => write!(f, "{fl}"),
            Self::U64(u) => write!(f, "{u}"),
            Self::U128(u) => write!(f, "{u}"),
            Self::I64(i) => write!(f, "{i}"),
            Self::I128(i) => write!(f, "{i}"),
            Self::Boolean(b) => write!(f, "{b}"),
        }
    }
}

impl LogLevel {
    pub fn style(&self) -> Style {
        let style = Style::new();

        match self {
            Self::Error => style.red(),
            Self::Warn => style.yellow(),
            Self::Info => style.green(),
            Self::Debug => style.blue(),
            Self::Trace => style.dim(),
        }
    }
}

impl From<tracing::Level> for LogLevel {
    fn from(value: tracing::Level) -> Self {
        match value {
            tracing::Level::ERROR => Self::Error,
            tracing::Level::WARN => Self::Warn,
            tracing::Level::INFO => Self::Info,
            tracing::Level::DEBUG => Self::Debug,
            tracing::Level::TRACE => Self::Trace,
        }
    }
}

impl Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let level = match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        };

        write!(f, "{level}")
    }
}

impl FromStr for LogLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "error" => Ok(Self::Error),
            "warn" => Ok(Self::Warn),
            "info" => Ok(Self::Info),
            "debug" => Ok(Self::Debug),
            "trace" => Ok(Self::Trace),
            _ => Err("unknown log level".into()),
        }
    }
}
