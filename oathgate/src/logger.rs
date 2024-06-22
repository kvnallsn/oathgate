//! A database logger

use std::{borrow::Cow, collections::BTreeMap, fmt::Display, path::Path, sync::{atomic::{AtomicU64, Ordering}, Arc}};

use console::Style;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::{span, Level, Metadata};
use uuid::Uuid;

use crate::database::{log::LogEntry, Database};

/// Where to save logs
pub enum LogDestination {
    Stdout,

    /// Save to the database and associate with specific uuid
    Database(Uuid),
}

/// A handle to settings that can be modified
#[derive(Clone)]
pub struct SubscriberHandle {
    dest: Arc<RwLock<LogDestination>>,
}

pub struct SqliteSubscriber {
    db: Database,
    max_level: Level,
    next_id: AtomicU64,
    dest: Arc<RwLock<LogDestination>>,
}

pub struct SqliteSubscriberBuilder {
    max_level: Level,
    dest: LogDestination,
}

#[derive(Debug)]
pub struct OathgateEvent<'a> {
    pub target: Cow<'a, str>,
    pub line: Option<u32>,
    pub module: Option<Cow<'a, str>>,
    pub level: Level,
    pub data: BTreeMap<Cow<'a, str>, DataType<'a>>,
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

    /// Finishes building the subscriber and installs it as the global default
    pub fn init<P: AsRef<Path>>(self, path: P) -> anyhow::Result<SubscriberHandle> {
        let db = Database::open(path)?;
        let dest = Arc::new(RwLock::new(self.dest));

        let subscriber = SqliteSubscriber {
            db, max_level: self.max_level,
            next_id: AtomicU64::new(0),
            dest: Arc::clone(&dest),
        };

        let handle = SubscriberHandle { dest };

        tracing::subscriber::set_global_default(subscriber)?;
        Ok(handle)
    }
}

impl SubscriberHandle {
    /// Change the destination for tracing events
    ///
    /// ### Arguments
    /// * `dest` - New tracing log/event/span destination
    pub fn set_destination(&self, dest: LogDestination) {
        *self.dest.write() = dest;
    }
}

impl SqliteSubscriber {
    /// Returns a new builder for our subscriber 
    pub fn builder() -> SqliteSubscriberBuilder {
        SqliteSubscriberBuilder { max_level: Level::ERROR, dest: LogDestination::Stdout }
    }
}

impl tracing::Subscriber for SqliteSubscriber {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        metadata.level() <= &self.max_level
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

        match *self.dest.read() {
            LogDestination::Stdout => {
                visitor.display();
            },
            LogDestination::Database(id) => {
                if let Err(error) = LogEntry::save(&self.db, id, &visitor) {
                    eprintln!("sqlite logger: {error}");
                }
            }
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
            name if name.starts_with("r#") => self.data.insert(name[2..].into(), DataType::from(dbg)),
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
        self.data.insert(field.name().into(), DataType::from(String::from(value)));
    }

    fn record_error(&mut self, field: &tracing::field::Field, value: &(dyn std::error::Error + 'static)) {
        let err = format!("{value}");
        self.data.insert(field.name().into(), DataType::from(err));
    }
}

impl<'a> OathgateEvent<'a> {
    pub fn new(metadata: &'a Metadata<'_>) -> Self {
        let data = BTreeMap::new();
        Self {
            target: metadata.target().into(),
            line: metadata.line(),
            module: metadata.module_path().map(|m| Cow::Borrowed(m)),
            level: *metadata.level(),
            data,
        }
    }

    pub fn display(&self) {
        let style = self.level_style();
        let dim = Style::new().dim();

        let pipe = format!("{}", style.apply_to("\u{251c}"));
        let arrow = format!("{}", style.apply_to("\u{2514}"));

        println!("{}: {}", style.apply_to(self.level), self.target);

        if let Some(msg) = self.data.get("message") {
            println!("{pipe} {msg}");
        }

        for (k, v) in &self.data {
            if *k == "message" {
                continue;
            }

            println!("{pipe} {} = {v}", dim.apply_to(k));
        }

        println!("{arrow} {}", self.target);
    }

    fn level_style(&self) -> Style {
        let style = Style::new();

        match self.level {
            Level::ERROR => style.red(),
            Level::WARN => style.yellow(),
            Level::INFO => style.green(),
            Level::DEBUG => style.blue(),
            Level::TRACE => style.white().dim(),
        }
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
