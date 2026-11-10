#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(windows)]
mod platform;
mod state;

use std::{ffi::OsString, path::PathBuf, process::ExitCode};

const HELP: &str = "WCCS 原生等待界面开发原型\n\n用法\n  wccs-session [--state-dir DIR]\n\n使用合成身份，每个显示器一个窗口。点击退出开发测试、Esc 或 Alt+F4 退出全部窗口。\n默认聚合记录保存在当前目录 local/waiting/session-<会话号>/status.json。\n重复启动退出 2，参数错误退出 2，平台或记录失败退出 1。\n";

fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Option<PathBuf>, String> {
    let args: Vec<_> = args.into_iter().collect();
    match args.as_slice() {
        [] => Ok(Some("local/waiting".into())),
        [help] if help == "--help" || help == "-h" => Ok(None),
        [option, path]
            if option == "--state-dir"
                && !path.is_empty()
                && !path.to_string_lossy().starts_with("--") =>
        {
            Ok(Some(path.into()))
        }
        _ => Err("参数无效".into()),
    }
}

fn main() -> ExitCode {
    let directory = match parse(std::env::args_os().skip(1)) {
        Ok(Some(directory)) => directory,
        Ok(None) => {
            print!("{HELP}");
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            eprintln!("{error}\n{HELP}");
            return ExitCode::from(2);
        }
    };
    #[cfg(windows)]
    match platform::run(&directory) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("等待界面运行失败，{error}");
            ExitCode::FAILURE
        }
    }
    #[cfg(not(windows))]
    {
        let _ = directory;
        eprintln!("需要 Windows 10 1703+ x64 用户会话");
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_accepts_explicit_path_and_rejects_extra_arguments() {
        assert_eq!(parse([]).unwrap(), Some(PathBuf::from("local/waiting")));
        assert!(parse(["--help".into()]).unwrap().is_none());
        assert_eq!(
            parse(["--state-dir".into(), "中文 空格".into()]).unwrap(),
            Some(PathBuf::from("中文 空格"))
        );
        for args in [
            vec!["--state-dir"],
            vec!["--unknown"],
            vec!["--state-dir", ""],
            vec!["--state-dir", "--help"],
        ] {
            assert!(parse(args.into_iter().map(OsString::from)).is_err());
        }
    }
}
