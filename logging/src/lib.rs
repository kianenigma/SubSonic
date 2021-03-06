use log::*;
use std::thread;

#[macro_export]
macro_rules! log_target {
	($target:tt, $level:tt, $patter:expr $(, $values:expr)* $(,)?) => {
		log::$level!(
			target: $target,
			$patter $(, $values)*
		)
	};
}

#[macro_export]
macro_rules! log {
	($level:tt, $patter:expr $(, $values:expr)* $(,)?) => {
		log::$level!(
			target: LOG_TARGET,
			$patter $(, $values)*
		)
	};
}

pub fn init_logger() {
	use colored::*;
	use std::io::Write;

	let _ = env_logger::Builder::from_env("RUST_LOG")
		.format(|buf, record| {
			writeln!(
				buf,
				"{} {} {} [{} ({:?})] - {}",
				chrono::Local::now()
					.format("%Y-%m-%dT%H:%M:%S")
					.to_string()
					.italic()
					.dimmed(),
				match record.level() {
					Level::Error => "Error".red(),
					Level::Warn => "Warn".yellow(),
					Level::Info => "Info".green(),
					Level::Debug => "Debug".magenta(),
					Level::Trace => "Trace".blue(),
				}
				.bold(),
				record.target().italic().cyan(),
				// record.module_path_static().unwrap_or("?"),
				thread::current().name().unwrap_or("Unnamed thread.").bold(),
				thread::current().id(),
				record.args()
			)
		})
		.try_init();
}
