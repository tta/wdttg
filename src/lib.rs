pub mod cli;
pub mod clipboard;
pub mod config;
pub mod db;
pub mod models;
pub mod services;
pub mod time_parser;
pub mod ui;

pub use cli::{Cli, Commands, ReportCommands};
pub use clipboard::{copy_to_clipboard, open_link, open_in_nvim, OpenLinkResult};
pub use db::{Database, DbError};
pub use models::{
    compute_trend_buckets, format_trend_arrow, quadrant_row_name, render_sparkline, BucketBy,
    EmQuadrant, QuadrantTime, Session, SessionSplitTime, SessionWithTask, TagTime, Task, TaskStatus,
    TaskTime, TrendDimension, TrendItem, TrendResult, SPARK_CHARS,
};
pub use services::{
    ReviewService, SessionService, SessionServiceError, TaskService, TaskServiceError, TimeBlock,
};
pub use time_parser::{parse_date, parse_period, parse_period_at, parse_relative_time, TimeParseError};
