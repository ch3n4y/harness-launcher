#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{BufRead, BufReader, Read},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager,
};
use wait_timeout::ChildExt;

const URL: &str = "http://127.0.0.1:3080";
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Config {
    node: String,
    cwd: String,
    auto_open: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            node: String::new(),
            cwd: dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .to_string_lossy()
                .into(),
            auto_open: true,
        }
    }
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Runtime {
    node: String,
    version: String,
    npm: String,
    root: String,
    prefix: String,
    harness_version: Option<String>,
    entry: Option<String>,
}
#[derive(Clone, Serialize)]
struct Snapshot {
    status: String,
    runtime: Option<Runtime>,
    candidates: Vec<String>,
    logs: Vec<String>,
    busy: bool,
}
struct Model {
    snapshot: Snapshot,
    config: Config,
    service: Option<Child>,
    installation: Option<Child>,
    generation: u64,
}
struct Shared {
    port: u16,
    model: Mutex<Model>,
    file: PathBuf,
    quitting: AtomicBool,
}
type SharedState = Arc<Shared>;
fn log(s: &SharedState, text: impl Into<String>) {
    let mut m = s.model.lock().unwrap();
    let text: String = text.into().chars().take(16000).collect();
    m.snapshot.logs.push(text);
    if m.snapshot.logs.len() > 1200 {
        m.snapshot.logs.remove(0);
    }
}
fn compatible(v: &str) -> bool {
    let parts: Vec<_> = v.trim().trim_start_matches('v').split('.').collect();
    if parts.len() < 3 {
        return false;
    }
    match (
        parts[0].parse::<u32>(),
        parts[1].parse::<u32>(),
        parts[2].parse::<u32>(),
    ) {
        (Ok(a), Ok(b), Ok(_)) => a >= 24 || (a == 22 && b >= 19),
        _ => false,
    }
}
fn command(node: &Path) -> Command {
    let mut c = Command::new(node);
    let mut paths = vec![node.parent().unwrap_or(Path::new(".")).to_path_buf()];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    if let Ok(value) = std::env::join_paths(paths) {
        c.env("PATH", value);
    }
    c.env_remove("ELECTRON_RUN_AS_NODE");
    c.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x08000000);
    }
    c
}
fn capture(mut c: Command) -> Result<String> {
    let mut child = c.spawn().map_err(|e| e.to_string())?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let read = |mut stream: Box<dyn Read + Send>| {
        let mut bytes = Vec::new();
        let _ = stream.by_ref().take(1024 * 1024).read_to_end(&mut bytes);
        String::from_utf8_lossy(&bytes).trim().to_string()
    };
    let out = thread::spawn(move || read(Box::new(stdout)));
    let err = thread::spawn(move || read(Box::new(stderr)));
    let status = child
        .wait_timeout(Duration::from_secs(25))
        .map_err(|e| e.to_string())?;
    if status.is_none() {
        let _ = child.kill();
        let _ = child.wait();
        return Err("环境检测命令超时".into());
    }
    let out = out.join().unwrap_or_default();
    let err = err.join().unwrap_or_default();
    if status.unwrap().success() {
        Ok(out)
    } else {
        Err(format!("命令失败：{err}"))
    }
}
fn candidates() -> Vec<String> {
    let executable = if cfg!(windows) { "node.exe" } else { "node" };
    let home = dirs::home_dir().unwrap_or_default();
    let mut paths: Vec<PathBuf> =
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|p| p.join(executable))
            .collect();
    if cfg!(windows) {
        for p in [
            "scoop/apps/nodejs-lts/current/node.exe",
            "scoop/apps/nodejs/current/node.exe",
        ] {
            paths.push(home.join(p));
        }
        paths.push(
            PathBuf::from(
                std::env::var_os("ProgramFiles").unwrap_or_else(|| "C:/Program Files".into()),
            )
            .join("nodejs/node.exe"),
        );
        if let Some(p) = std::env::var_os("NVM_SYMLINK") {
            paths.push(PathBuf::from(p).join(executable));
        }
    } else {
        for p in [
            "/opt/homebrew/bin/node",
            "/usr/local/bin/node",
            "/usr/bin/node",
        ] {
            paths.push(p.into());
        }
    }
    for (root, suffix) in [
        (home.join(".nvm/versions/node"), "bin/node"),
        (
            home.join(".local/share/fnm/node-versions"),
            "installation/bin/node",
        ),
        (
            home.join("AppData/Roaming/fnm/node-versions"),
            "installation/node.exe",
        ),
    ] {
        if let Ok(entries) = fs::read_dir(root) {
            let mut versions: Vec<_> = entries.flatten().map(|e| e.path().join(suffix)).collect();
            versions.sort();
            versions.reverse();
            paths.extend(versions);
        }
    }
    let mut result = Vec::new();
    for p in paths {
        if p.is_file() {
            let p = p.to_string_lossy().to_string();
            if !result.contains(&p) {
                result.push(p);
            }
        }
    }
    result
}
fn inspect(node: &str, cwd: &str) -> Result<Runtime> {
    let node_path = PathBuf::from(node);
    if !node_path.is_absolute() || !node_path.is_file() {
        return Err("请选择有效的 Node 可执行文件绝对路径".into());
    }
    let mut c = command(&node_path);
    c.arg("--version").current_dir(cwd);
    let version = capture(c)?;
    if !compatible(&version) {
        return Err(format!("Node {version} 不兼容，需要 22.19+（22.x）或 24+"));
    }
    let real = fs::canonicalize(&node_path).map_err(|e| e.to_string())?;
    let mut npm_paths = Vec::new();
    for p in [&node_path, &real] {
        let dir = p.parent().unwrap();
        npm_paths.push(dir.join("node_modules/npm/bin/npm-cli.js"));
        npm_paths.push(dir.join("../lib/node_modules/npm/bin/npm-cli.js"));
    }
    let npm = npm_paths
        .into_iter()
        .find(|p| p.is_file())
        .ok_or("找不到该 Node 配套的 npm，请选择完整的 Node 安装")?;
    let query = |args: &[&str]| {
        let mut c = command(&node_path);
        c.arg(&npm).args(args).current_dir(cwd);
        capture(c)
    };
    let root = query(&["root", "-g"])?;
    let prefix = query(&["prefix", "-g"])?;
    let package = Path::new(&root).join("@deepseek-ai/dsh");
    let mut harness_version = None;
    let mut entry = None;
    if package.join("package.json").is_file() {
        let manifest: serde_json::Value = serde_json::from_slice(
            &fs::read(package.join("package.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let bin = manifest["bin"]
            .as_str()
            .or_else(|| manifest["bin"]["dsh"].as_str())
            .ok_or("Harness 缺少启动入口，请重新安装")?;
        let file = package.join(bin);
        if !file.is_file() {
            return Err("Harness 入口文件缺失，请重新安装".into());
        }
        harness_version = manifest["version"].as_str().map(String::from);
        entry = Some(file.to_string_lossy().into());
    }
    Ok(Runtime {
        node: node.into(),
        version,
        npm: npm.to_string_lossy().into(),
        root,
        prefix,
        harness_version,
        entry,
    })
}
fn detect(s: &SharedState) -> Result<()> {
    let config = s.model.lock().unwrap().config.clone();
    let found = candidates();
    let paths = if config.node.is_empty() {
        found.clone()
    } else {
        vec![config.node]
    };
    {
        let mut m = s.model.lock().unwrap();
        m.snapshot.candidates = found;
        m.snapshot.runtime = None;
    }
    let mut error = "未找到 Node，请安装或手动选择".to_string();
    for node in paths {
        match inspect(&node, &config.cwd) {
            Ok(rt) => {
                s.model.lock().unwrap().snapshot.runtime = Some(rt);
                return Ok(());
            }
            Err(e) => error = e,
        }
    }
    Err(error)
}
fn pipe(s: &SharedState, child: &mut Child) {
    fn stream(s: SharedState, input: impl Read + Send + 'static) {
        thread::spawn(move || {
            let mut reader = BufReader::new(input);
            let mut pending = Vec::new();
            loop {
                let available = match reader.fill_buf() {
                    Ok(b) if !b.is_empty() => b,
                    _ => break,
                };
                let count = available.len();
                for byte in available {
                    if *byte == b'\n' || pending.len() >= 8192 {
                        log(&s, String::from_utf8_lossy(&pending).to_string());
                        pending.clear();
                    }
                    if *byte != b'\n' {
                        pending.push(*byte);
                    }
                }
                reader.consume(count);
            }
            if !pending.is_empty() {
                log(&s, String::from_utf8_lossy(&pending).to_string());
            }
        });
    }
    stream(s.clone(), child.stdout.take().unwrap());
    stream(s.clone(), child.stderr.take().unwrap());
}
fn kill(child: &mut Child) -> Result<()> {
    if child.try_wait().map_err(|e| e.to_string())?.is_some() {
        return Ok(());
    }
    #[cfg(windows)]
    {
        let taskkill =
            PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:/Windows".into()))
                .join("System32/taskkill.exe");
        let mut c = command(&taskkill);
        c.args(["/PID", &child.id().to_string(), "/T", "/F"]);
        capture(c)?;
    }
    #[cfg(unix)]
    {
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGTERM);
        }
        let _ = child.wait_timeout(Duration::from_secs(3));
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
    }
    child
        .wait_timeout(Duration::from_secs(5))
        .map_err(|e| e.to_string())?
        .ok_or("等待进程退出超时")?;
    Ok(())
}
fn stop(s: &SharedState) -> Result<()> {
    let mut m = s.model.lock().unwrap();
    if m.service.is_none() {
        return Ok(());
    }
    m.snapshot.status = "stopping".into();
    if let Some(child) = m.service.as_mut() {
        kill(child)?;
    }
    m.service = None;
    m.generation += 1;
    m.snapshot.status = "stopped".into();
    drop(m);
    log(s, "服务已停止");
    Ok(())
}
fn start(s: &SharedState) -> Result<()> {
    {
        let m = s.model.lock().unwrap();
        if m.service.is_some() || m.installation.is_some() {
            return Err("已有任务正在运行".into());
        }
    }
    detect(s)?;
    let mut m = s.model.lock().unwrap();
    if s.quitting.load(Ordering::SeqCst) {
        return Err("启动器正在退出".into());
    }
    let rt = m.snapshot.runtime.clone().ok_or("运行环境不可用")?;
    let entry = rt.entry.ok_or("请先安装 Harness")?;
    let listener = TcpListener::bind(("127.0.0.1", s.port))
        .map_err(|_| format!("{} 端口已占用，请停止已有服务；也可点击打开浏览器", s.port))?;
    drop(listener);
    let mut c = command(Path::new(&rt.node));
    c.args([&entry, "web", "--no-open", "--port", &s.port.to_string()])
        .current_dir(&m.config.cwd);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.process_group(0);
    }
    let mut child = c.spawn().map_err(|e| e.to_string())?;
    pipe(s, &mut child);
    m.service = Some(child);
    m.generation += 1;
    let generation = m.generation;
    let auto_open = m.config.auto_open;
    m.snapshot.status = "starting".into();
    drop(m);
    log(s, "正在启动 Harness，等待服务就绪…");
    let s = s.clone();
    thread::spawn(move || {
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(120);
        let mut ready = false;
        loop {
            {
                let mut m = s.model.lock().unwrap();
                if m.generation != generation {
                    return;
                }
                let Some(child) = m.service.as_mut() else {
                    return;
                };
                match child.try_wait() {
                    Ok(Some(code)) => {
                        m.service = None;
                        m.snapshot.status = if code.success() { "stopped" } else { "error" }.into();
                        drop(m);
                        log(&s, format!("服务已退出：{code}"));
                        return;
                    }
                    Err(e) => {
                        drop(m);
                        log(&s, e.to_string());
                        return;
                    }
                    _ => {}
                }
            }
            if !ready {
                if client
                    .get(format!("http://127.0.0.1:{}", s.port))
                    .send()
                    .map(|r| r.status().is_success())
                    .unwrap_or(false)
                {
                    let mut m = s.model.lock().unwrap();
                    if m.generation != generation {
                        return;
                    }
                    m.snapshot.status = "running".into();
                    ready = true;
                    drop(m);
                    log(&s, "服务已就绪：http://127.0.0.1:3080");
                    if auto_open {
                        if let Err(e) = open::that(URL) {
                            log(&s, e.to_string());
                        }
                    }
                } else if Instant::now() >= deadline {
                    // Check and stop under the same lock so a newer service is never killed.
                    let mut m = s.model.lock().unwrap();
                    if m.generation != generation {
                        return;
                    }
                    if let Some(child) = m.service.as_mut() {
                        if let Err(e) = kill(child) {
                            drop(m);
                            log(&s, e);
                            return;
                        }
                    }
                    m.service = None;
                    m.generation += 1;
                    m.snapshot.status = "error".into();
                    drop(m);
                    log(&s, "服务在 120 秒内未就绪，已停止。请检查日志。");
                    return;
                }
            }
            thread::sleep(Duration::from_millis(700));
        }
    });
    Ok(())
}
fn install(s: &SharedState) -> Result<()> {
    {
        let m = s.model.lock().unwrap();
        if m.service.is_some() || m.installation.is_some() {
            return Err("请先停止服务".into());
        }
    }
    detect(s)?;
    let mut m = s.model.lock().unwrap();
    if s.quitting.load(Ordering::SeqCst) {
        return Err("启动器正在退出".into());
    }
    let rt = m.snapshot.runtime.clone().unwrap();
    let mut c = command(Path::new(&rt.node));
    c.arg(&rt.npm)
        .args([
            "install",
            "-g",
            "@deepseek-ai/dsh@latest",
            "--no-audit",
            "--no-fund",
        ])
        .current_dir(&m.config.cwd);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.process_group(0);
    }
    let mut child = c.spawn().map_err(|e| e.to_string())?;
    pipe(s, &mut child);
    m.installation = Some(child);
    m.snapshot.status = "installing".into();
    drop(m);
    log(s, format!("npm 全局安装 / 更新，目标：{}", rt.root));
    let outcome = loop {
        let mut m = s.model.lock().unwrap();
        let Some(child) = m.installation.as_mut() else {
            break Err("安装已取消".into());
        };
        match child.try_wait() {
            Ok(Some(code)) => {
                m.installation = None;
                break if code.success() {
                    Ok(())
                } else {
                    Err(format!(
                        "npm 安装失败：{code}。请查看日志；权限不足时修复 npm 全局目录权限。"
                    ))
                };
            }
            Err(e) => break Err(e.to_string()),
            _ => {}
        }
        drop(m);
        thread::sleep(Duration::from_millis(250));
    };
    s.model.lock().unwrap().snapshot.status = "stopped".into();
    outcome?;
    detect(s)?;
    log(s, "安装完成，终端可使用同一 npm 环境中的 dsh");
    Ok(())
}
fn dispatch(s: &SharedState, action: &str, data: serde_json::Value) -> Result<serde_json::Value> {
    if action == "state" {
        let m = s.model.lock().unwrap();
        return Ok(serde_json::json!({"state":m.snapshot,"config":m.config}));
    }
    if action == "open" {
        open::that(URL).map_err(|e| e.to_string())?;
        return Ok(serde_json::Value::Null);
    }
    if action == "clear" {
        s.model.lock().unwrap().snapshot.logs.clear();
        return Ok(serde_json::Value::Null);
    }
    if action == "pickNode" || action == "pickCwd" {
        let dialog = rfd::FileDialog::new();
        let p = if action == "pickNode" {
            dialog.pick_file()
        } else {
            dialog.pick_folder()
        };
        return Ok(p
            .map(|p| serde_json::json!(p.to_string_lossy()))
            .unwrap_or_default());
    }
    if action == "export" {
        if let Some(p) = rfd::FileDialog::new()
            .set_file_name("harness.log")
            .save_file()
        {
            fs::write(p, s.model.lock().unwrap().snapshot.logs.join("\n"))
                .map_err(|e| e.to_string())?;
        }
        return Ok(serde_json::Value::Null);
    }
    {
        let mut m = s.model.lock().unwrap();
        if s.quitting.load(Ordering::SeqCst) {
            return Err("启动器正在退出".into());
        }
        if m.snapshot.busy {
            return Err("正在执行操作，请稍候".into());
        }
        m.snapshot.busy = true;
    }
    let result: Result<()> = (|| match action {
        "detect" => detect(s),
        "start" => start(s),
        "stop" => stop(s),
        "restart" => {
            stop(s)?;
            start(s)
        }
        "install" => install(s),
        "save" => {
            let cfg: Config = serde_json::from_value(data).map_err(|e| e.to_string())?;
            if !Path::new(&cfg.cwd).is_absolute() || !Path::new(&cfg.cwd).is_dir() {
                return Err("请选择有效的工作目录绝对路径".into());
            }
            {
                let mut m = s.model.lock().unwrap();
                if m.service.is_some() || m.installation.is_some() {
                    return Err("请先停止服务再保存设置".into());
                }
                fs::write(&s.file, serde_json::to_vec_pretty(&cfg).unwrap())
                    .map_err(|e| e.to_string())?;
                m.config = cfg;
            }
            detect(s)
        }
        _ => Err("未知操作".into()),
    })();
    s.model.lock().unwrap().snapshot.busy = false;
    result.map(|_| serde_json::Value::Null)
}
#[tauri::command]
async fn action(
    state: tauri::State<'_, SharedState>,
    name: String,
    data: Option<serde_json::Value>,
) -> Result<serde_json::Value> {
    let s = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let result = dispatch(&s, &name, data.unwrap_or_default());
        if let Err(e) = &result {
            log(&s, e.clone());
        }
        result
    })
    .await
    .map_err(|e| e.to_string())?
}
fn show(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}
fn quit(app: tauri::AppHandle, s: SharedState) {
    if s.quitting.swap(true, Ordering::SeqCst) {
        return;
    }
    thread::spawn(move || {
        let outcome = (|| {
            stop(&s)?;
            let mut m = s.model.lock().unwrap();
            if let Some(child) = m.installation.as_mut() {
                kill(child)?;
            }
            m.installation = None;
            Ok::<_, String>(())
        })();
        match outcome {
            Ok(()) => app.exit(0),
            Err(e) => {
                s.quitting.store(false, Ordering::SeqCst);
                log(&s, format!("退出失败：{e}"));
                show(&app);
            }
        }
    });
}
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| show(app)))
        .setup(|app| {
            let dir = app.path().app_config_dir()?;
            fs::create_dir_all(&dir)?;
            let file = dir.join("config.json");
            let config = fs::read(&file)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default();
            let state = Arc::new(Shared {
                port: 3080,
                model: Mutex::new(Model {
                    snapshot: Snapshot {
                        status: "stopped".into(),
                        runtime: None,
                        candidates: vec![],
                        logs: vec![],
                        busy: false,
                    },
                    config,
                    service: None,
                    installation: None,
                    generation: 0,
                }),
                file,
                quitting: AtomicBool::new(false),
            });
            app.manage(state.clone());
            let items = [
                ("show", "显示窗口"),
                ("open", "打开浏览器"),
                ("start", "启动服务"),
                ("stop", "停止服务"),
                ("restart", "重启服务"),
                ("quit", "退出"),
            ];
            let menu = Menu::new(app)?;
            for (id, title) in items {
                menu.append(&MenuItem::with_id(app, id, title, true, None::<&str>)?)?;
            }
            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Harness Launcher")
                .menu(&menu)
                .on_menu_event(|app, event| {
                    if event.id.as_ref() == "show" {
                        show(app);
                        return;
                    }
                    let s = app.state::<SharedState>().inner().clone();
                    if event.id.as_ref() == "quit" {
                        quit(app.clone(), s);
                        return;
                    }
                    let name = event.id.as_ref().to_string();
                    thread::spawn(move || {
                        if let Err(e) = dispatch(&s, &name, serde_json::Value::Null) {
                            log(&s, e);
                        }
                    });
                })
                .build(app)?;
            thread::spawn(move || {
                if let Err(e) = dispatch(&state, "detect", serde_json::Value::Null) {
                    log(&state, e);
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![action])
        .build(tauri::generate_context!())
        .expect("无法初始化启动器")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                let s = app.state::<SharedState>().inner().clone();
                if !s.quitting.load(Ordering::SeqCst) {
                    api.prevent_exit();
                    quit(app.clone(), s);
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn version_range() {
        for v in ["v22.19.0", "22.20.1", "24.0.0", "25.1.0"] {
            assert!(compatible(v));
        }
        for v in ["22.18.9", "23.9.0", "20.1.0", "foo", "24"] {
            assert!(!compatible(v));
        }
    }
    #[test]
    fn node_first_on_path() {
        let exe = std::env::current_exe().unwrap();
        let c = command(&exe);
        let env = c.get_envs().find(|(k, _)| *k == "PATH").unwrap().1.unwrap();
        assert_eq!(
            std::env::split_paths(env).next().unwrap(),
            exe.parent().unwrap()
        );
    }
    #[test]
    fn bounded_logs() {
        let s = Arc::new(Shared {
            model: Mutex::new(Model {
                snapshot: Snapshot {
                    status: "stopped".into(),
                    runtime: None,
                    candidates: vec![],
                    logs: vec![],
                    busy: false,
                },
                config: Config::default(),
                service: None,
                installation: None,
                generation: 0,
            }),
            port: 3080,
            file: PathBuf::new(),
            quitting: AtomicBool::new(false),
        });
        for _ in 0..1250 {
            log(&s, "测试日志");
        }
        assert_eq!(s.model.lock().unwrap().snapshot.logs.len(), 1200);
    }
    #[test]
    fn npm_install_and_service_lifecycle_in_isolated_fixture() {
        let original = candidates()
            .into_iter()
            .find(|p| {
                let mut c = command(Path::new(p));
                c.arg("--version");
                capture(c).map(|v| compatible(&v)).unwrap_or(false)
            })
            .expect("Integration test requires a compatible Node on this machine");
        let root =
            std::env::temp_dir().join(format!("harness launcher 测试 {}", std::process::id()));
        fs::create_dir_all(root.join("node_modules/npm/bin")).unwrap();
        let node = root.join(if cfg!(windows) { "node.exe" } else { "node" });
        fs::copy(original, &node).unwrap();
        let npm = r#"const fs=require('fs'),p=require('path');const root=p.resolve(__dirname,'../../..');const global=p.join(root,'global');const args=process.argv.slice(2);if(args[0]==='root'){console.log(global)}else if(args[0]==='prefix'){console.log(root)}else if(args[0]==='install'){fs.writeFileSync(p.join(root,'install-args.json'),JSON.stringify(args));const dir=p.join(global,'@deepseek-ai/dsh');fs.mkdirSync(dir,{recursive:true});fs.writeFileSync(p.join(dir,'package.json'),JSON.stringify({version:'0.0.0-fixture',bin:{dsh:'bin.js'}}));fs.writeFileSync(p.join(dir,'bin.js'),`require('fs').writeFileSync('service-args.json',JSON.stringify(process.argv.slice(2)));require('http').createServer((q,s)=>s.end('fixture')).listen(Number(process.argv[5]),'127.0.0.1');console.log('fixture ready');`)}else{process.exit(2)}"#;
        fs::write(root.join("node_modules/npm/bin/npm-cli.js"), npm).unwrap();
        let s = Arc::new(Shared {
            model: Mutex::new(Model {
                snapshot: Snapshot {
                    status: "stopped".into(),
                    runtime: None,
                    candidates: vec![],
                    logs: vec![],
                    busy: false,
                },
                config: Config {
                    node: node.to_string_lossy().into(),
                    cwd: root.to_string_lossy().into(),
                    auto_open: false,
                },
                service: None,
                installation: None,
                generation: 0,
            }),
            port: TcpListener::bind("127.0.0.1:0")
                .unwrap()
                .local_addr()
                .unwrap()
                .port(),
            file: root.join("config.json"),
            quitting: AtomicBool::new(false),
        });
        struct Cleanup(SharedState, PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = stop(&self.0);
                let _ = fs::remove_dir_all(&self.1);
            }
        }
        let _cleanup = Cleanup(s.clone(), root.clone());
        dispatch(&s, "detect", serde_json::Value::Null).unwrap();
        assert!(s
            .model
            .lock()
            .unwrap()
            .snapshot
            .runtime
            .as_ref()
            .unwrap()
            .entry
            .is_none());
        dispatch(&s, "install", serde_json::Value::Null).unwrap();
        let args: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("install-args.json")).unwrap()).unwrap();
        assert_eq!(args[0], "install");
        assert_eq!(args[1], "-g");
        assert_eq!(args[2], "@deepseek-ai/dsh@latest");
        assert_eq!(
            s.model
                .lock()
                .unwrap()
                .snapshot
                .runtime
                .as_ref()
                .unwrap()
                .harness_version
                .as_deref(),
            Some("0.0.0-fixture")
        );
        let port = TcpListener::bind(("127.0.0.1", s.port)).unwrap();
        assert!(dispatch(&s, "start", serde_json::Value::Null)
            .unwrap_err()
            .contains("端口已占用"));
        drop(port);
        dispatch(&s, "start", serde_json::Value::Null).unwrap();
        assert!(dispatch(&s, "start", serde_json::Value::Null).is_err());
        let deadline = Instant::now() + Duration::from_secs(15);
        while s.model.lock().unwrap().snapshot.status != "running" && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(s.model.lock().unwrap().snapshot.status, "running");
        assert_eq!(
            fs::read_to_string(root.join("service-args.json")).unwrap(),
            format!(r#"["web","--no-open","--port","{}"]"#, s.port)
        );
        assert!(dispatch(&s, "install", serde_json::Value::Null).is_err());
        dispatch(&s, "restart", serde_json::Value::Null).unwrap();
        dispatch(&s, "stop", serde_json::Value::Null).unwrap();
        assert_eq!(s.model.lock().unwrap().snapshot.status, "stopped");
        assert!(TcpListener::bind(("127.0.0.1", s.port)).is_ok());
    }
}
