//! 自动测试用的探测程序。以隔离账户身份运行，逐项尝试读写，把结果写进交换文件夹。
//!
//! 用法：probe.exe <交换文件夹> [要测试读取的受保护盘上的文件]
//! 输出：<交换文件夹>\probe-result.txt，每行 名字=结果

use std::path::{Path, PathBuf};

fn read(name: &str, p: &str) -> String {
    match std::fs::read(p) {
        Ok(_) => format!("{name}=READ_OK"),
        Err(e) => format!("{name}=DENIED({:?})", e.kind()),
    }
}

fn list(name: &str, p: &str) -> String {
    match std::fs::read_dir(p) {
        Ok(_) => format!("{name}=LIST_OK"),
        Err(e) => format!("{name}=DENIED({:?})", e.kind()),
    }
}

fn write(name: &str, p: &Path) -> String {
    match std::fs::write(p, b"x") {
        Ok(_) => {
            let _ = std::fs::remove_file(p);
            format!("{name}=WRITE_OK")
        }
        Err(e) => format!("{name}=DENIED({:?})", e.kind()),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = PathBuf::from(args.get(1).cloned().unwrap_or_else(|| ".".into()));
    let mut lines = vec![format!("user={}", std::env::var("USERNAME").unwrap_or_default())];

    lines.push(read("read_protected_file", r"C:\TestData\secret.txt"));
    lines.push(list("list_protected_dir", r"C:\TestData"));
    lines.push(read("read_main_profile", r"C:\Users\WDAGUtilityAccount\Desktop\secret.txt"));
    if let Some(t) = args.get(2) {
        lines.push(read("read_protected_drive", t));
    }

    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
    if let Some(d) = &exe_dir {
        lines.push(write("write_own_app_dir", &d.join("probe-write.tmp")));
        lines.push(format!("exe_dir={}", d.display()));
    }
    lines.push(write("write_install_root", Path::new(r"C:\Program Files\IsoBox\probe-write.tmp")));

    // 在 ProgramData 里留下一个文件夹，测试卸载时能否按所有者找到并清理
    let leftover = Path::new(r"C:\ProgramData\IsoProbeLeftover");
    lines.push(match std::fs::create_dir_all(leftover).and_then(|_| std::fs::write(leftover.join("x.txt"), b"x")) {
        Ok(_) => "create_programdata_leftover=OK".into(),
        Err(e) => format!("create_programdata_leftover=FAILED({:?})", e.kind()),
    });

    let result = out.join("probe-result.txt");
    lines.push(match std::fs::write(&result, lines.join("\n")) {
        Ok(_) => "write_exchange=WRITE_OK".into(),
        Err(e) => format!("write_exchange=DENIED({:?})", e.kind()),
    });
    // 写两次：第二次带上 write_exchange 本身的结果
    let _ = std::fs::write(&result, lines.join("\n"));
}
