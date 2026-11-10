#![forbid(unsafe_code)]

use std::{ffi::OsString, path::PathBuf, process::ExitCode};
use wccs_inventory::model::EvidenceContext;

const HELP: &str = "WCCS Windows 只读环境采集\n\n用法\n  wccs-inventory [--output-dir DIR] [--context development|lab] [--tool-dir DIR]...\n\n默认输出 local/environment，默认标记 development 开发环境证据。\n每次运行新建报告目录，含完整 report.json 与脱敏 summary.txt。\n--tool-dir 追加工具文件搜索目录。工具探测仅读取文件及 PE 版本资源。\n部分字段失败仍导出报告并退出 0，参数错误退出 2，采集或导出失败退出 1。\n";

#[derive(Debug)]
struct Options {
    output: PathBuf,
    context: EvidenceContext,
    tool_directories: Vec<PathBuf>,
}

fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Option<Options>, String> {
    let mut options = Options {
        output: "local/environment".into(),
        context: EvidenceContext::Development,
        tool_directories: Vec::new(),
    };
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        if argument == "--help" || argument == "-h" {
            return Ok(None);
        }
        if argument != "--output-dir" && argument != "--context" && argument != "--tool-dir" {
            return Err(format!("未知参数 {}", argument.to_string_lossy()));
        }
        let value = arguments
            .next()
            .filter(|value| !value.is_empty() && !value.to_string_lossy().starts_with("--"))
            .ok_or_else(|| format!("参数 {} 缺少值", argument.to_string_lossy()))?;
        if argument == "--output-dir" {
            options.output = value.into();
        } else if argument == "--tool-dir" {
            options.tool_directories.push(value.into());
        } else {
            options.context = match value.to_str() {
                Some("development") => EvidenceContext::Development,
                Some("lab") => EvidenceContext::Lab,
                _ => return Err("--context 必须为 development 或 lab".into()),
            };
        }
    }
    Ok(Some(options))
}

fn main() -> ExitCode {
    let options = match parse(std::env::args_os().skip(1)) {
        Ok(Some(options)) => options,
        Ok(None) => {
            print!("{HELP}");
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            eprintln!("{error}\n{HELP}");
            return ExitCode::from(2);
        }
    };
    match run(options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("采集工具执行失败，{error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(windows)]
fn run(options: Options) -> std::io::Result<()> {
    let report = wccs_inventory::collect(options.context, &options.tool_directories)?;
    let destination = wccs_inventory::output::export(&report, &options.output)?;
    // Output the sanitized summary after the complete bundle has been published.
    use std::io::Write;
    let mut stdout = std::io::stdout().lock();
    writeln!(
        stdout,
        "{}",
        wccs_inventory::output::summary(&report).map_err(std::io::Error::other)?
    )?;
    writeln!(stdout, "报告目录 {}", destination.display())
}

#[cfg(not(windows))]
fn run(_options: Options) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "本切片的真实采集需要 Windows 10 x64 或更新系统",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_context_is_development() {
        let options = parse([]).unwrap().unwrap();
        assert_eq!(options.context, EvidenceContext::Development);
        assert_eq!(options.output, PathBuf::from("local/environment"));
    }

    #[test]
    fn invalid_options_fail_before_collection() {
        for arguments in [
            vec!["--context", "production"],
            vec!["--output-dir"],
            vec!["--tool-dir", "--context"],
            vec!["--delete"],
        ] {
            assert!(parse(arguments.into_iter().map(OsString::from)).is_err());
        }
    }

    #[test]
    fn paths_with_spaces_and_lab_context_are_preserved() {
        let options = parse(
            [
                "--output-dir",
                "中文 空格",
                "--context",
                "lab",
                "--tool-dir",
                "工具 目录",
                "--tool-dir",
                "tools",
            ]
            .map(OsString::from),
        )
        .unwrap()
        .unwrap();
        assert_eq!(options.output, PathBuf::from("中文 空格"));
        assert_eq!(options.context, EvidenceContext::Lab);
        assert_eq!(options.tool_directories.len(), 2);
    }
}
