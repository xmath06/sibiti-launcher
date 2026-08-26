use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use serde::Serialize;
use tauri::Emitter;
use tauri::Manager;

// CREATE_NO_WINDOW (0x08000000): jalankan child process tanpa jendela CMD hitam (Windows).
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Serialize)]
pub struct DiscoveryResult {
    pub backend: Option<String>,
    pub frontend: Option<String>,
}

#[derive(Serialize, serde::Deserialize)]
struct ConfigPaths {
    backend: Option<String>,
    frontend: Option<String>,
}

#[derive(Serialize)]
pub struct StatusResult {
    pub is_postgres_installed: bool,
    pub is_postgres_running: bool,
    pub is_pm2_installed: bool,
    pub are_deps_installed: bool,
    pub is_frontend_built: bool,
    pub backend_path: Option<String>,
    pub frontend_path: Option<String>,
}

#[derive(Serialize)]
pub struct RunningStatus {
    pub frontend: bool,
    pub backend: bool,
}

// ───────────────────────── Auto-Discovery ─────────────────────────

fn read_package_deps(path: &Path) -> Option<serde_json::Value> {
    let p = path.join("package.json");
    let s = fs::read_to_string(p).ok()?;
    serde_json::from_str(&s).ok()
}

fn looks_like_backend(path: &Path) -> bool {
    if let Some(v) = read_package_deps(path) {
        let deps = v.get("dependencies").cloned().unwrap_or(serde_json::Value::Null);
        let scripts = v.get("scripts").cloned().unwrap_or(serde_json::Value::Null);
        let has_elysia = deps.get("elysia").is_some();
        let has_migrate = scripts.get("db:migrate").is_some();
        return has_elysia || has_migrate;
    }
    false
}

fn looks_like_frontend(path: &Path) -> bool {
    if let Some(v) = read_package_deps(path) {
        let deps = v.get("dependencies").cloned().unwrap_or(serde_json::Value::Null);
        let dev = v.get("devDependencies").cloned().unwrap_or(serde_json::Value::Null);
        // CBT frontend = SvelteKit. Hindari kecocokan dengan folder launcher sendiri.
        return deps.get("@sveltejs/kit").is_some() || dev.get("@sveltejs/kit").is_some();
    }
    false
}

/// Path absolut (canonical) sebagai String, fallback ke path asli jika gagal.
fn canon(p: &Path) -> String {
    p.canonicalize()
        .unwrap_or_else(|_| p.to_path_buf())
        .to_string_lossy()
        .to_string()
}

fn resolve_relative(base: &Path, rel: &str) -> String {
    let p = Path::new(rel);
    if p.is_absolute() {
        return rel.to_string();
    }
    canon(&base.join(rel))
}

fn read_config() -> Option<ConfigPaths> {
    for rel in ["./launcher.config.json", "../launcher.config.json", "launcher.config.json"] {
        let p = Path::new(rel);
        if let Ok(s) = fs::read_to_string(p) {
            if let Ok(mut v) = serde_json::from_str::<ConfigPaths>(&s) {
                let base = p.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
                if let Some(b) = v.backend {
                    v.backend = Some(resolve_relative(&base, &b));
                }
                if let Some(f) = v.frontend {
                    v.frontend = Some(resolve_relative(&base, &f));
                }
                return Some(v);
            }
        }
    }
    None
}

/// Cari proxy-server.mjs: di resource ter-bundle (hasil build), di folder
/// launcher (mode dev), atau di rantai parent dari exe. Mengembalikan path
/// absolut pertama yang benar-benar ada.
fn find_proxy(app: &tauri::AppHandle, frontend: &str) -> Option<String> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(rd) = app.path().resource_dir() {
        candidates.push(rd.join("proxy-server.mjs"));
    }
    if let Some(p) = Path::new(frontend).parent() {
        candidates.push(p.join("launcher").join("proxy-server.mjs"));
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut p = Some(exe.clone());
        while let Some(cur) = p {
            candidates.push(cur.join("proxy-server.mjs"));
            p = cur.parent().map(|x| x.to_path_buf());
        }
    }
    candidates.push(PathBuf::from("proxy-server.mjs"));
    for c in &candidates {
        if c.exists() {
            return Some(c.to_string_lossy().to_string());
        }
    }
    None
}

fn find_project_dirs() -> DiscoveryResult {
    // Cari di: rantai parent dari exe, cwd, dan parent/grandparent dari cwd.
    // (backend/frontend biasanya bersaudara dengan folder launcher, di bawah
    //  folder induk, mis. cbt/{backend,frontend,launcher}.)
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        let mut p = exe.parent().map(|x| x.to_path_buf());
        while let Some(cur) = p {
            roots.push(cur.clone());
            p = cur.parent().map(|x| x.to_path_buf());
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd.clone());
        if let Some(parent) = cwd.parent() {
            roots.push(parent.to_path_buf());
            if let Some(gp) = parent.parent() {
                roots.push(gp.to_path_buf());
            }
        }
    }
    roots.dedup();

    let mut backend = None;
    let mut frontend = None;
    for root in &roots {
        if backend.is_some() && frontend.is_some() {
            break;
        }
        if let Ok(entries) = fs::read_dir(root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                if backend.is_none() && looks_like_backend(&path) {
                    backend = Some(canon(&path));
                }
                if frontend.is_none() && looks_like_frontend(&path) {
                    frontend = Some(canon(&path));
                }
            }
        }
    }

    if backend.is_none() || frontend.is_none() {
        if let Some(cfg) = read_config() {
            if backend.is_none() {
                backend = cfg.backend;
            }
            if frontend.is_none() {
                frontend = cfg.frontend;
            }
        }
    }

    DiscoveryResult { backend, frontend }
}

/// Salin direktori secara rekursif (tanpa mengikuti symlink).
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let dest = dst.join(entry.file_name());
        if path.is_dir() {
            copy_dir_recursive(&path, &dest)?;
        } else {
            fs::copy(&path, &dest)?;
        }
    }
    Ok(())
}

/// Mode ter-bundle (installer self-contained): backend/frontend di-bundle ke
/// resource aplikasi. Pada jalankan pertama, salin ke direktori data aplikasi
/// yang writable, lalu kembalikan path tersebut agar langkah install/run
/// berikutnya bekerja di folder tersebut.
fn extract_bundled(app: &tauri::AppHandle) -> Option<(String, String)> {
    let rd = app.path().resource_dir().ok()?;
    // Cari backend/frontend ter-bundle berdasarkan tanda package.json-nya
    // (elysia / db:migrate untuk backend, @sveltejs/kit untuk frontend),
    // terlepas di folder mana Tauri meletakkannya di resource_dir.
    let mut backend_src = None;
    let mut frontend_src = None;
    let roots = [rd.clone(), rd.join("app"), rd.join("resources")];
    for root in roots.iter() {
        if !root.is_dir() {
            continue;
        }
        if let Ok(entries) = fs::read_dir(root) {
            for entry in entries.flatten() {
                let p = entry.path();
                if !p.is_dir() {
                    continue;
                }
                if backend_src.is_none() && looks_like_backend(&p) {
                    backend_src = Some(p.clone());
                }
                if frontend_src.is_none() && looks_like_frontend(&p) {
                    frontend_src = Some(p.clone());
                }
            }
        }
        if backend_src.is_some() && frontend_src.is_some() {
            break;
        }
    }
    let (b_src, f_src) = (backend_src?, frontend_src?);
    let data = app.path().app_data_dir().ok()?;
    let base = data.join("cbt-app");
    let b_dst = base.join("backend");
    let f_dst = base.join("frontend");
    if !b_dst.join("package.json").exists() {
        let _ = fs::remove_dir_all(&b_dst);
        if copy_dir_recursive(&b_src, &b_dst).is_err() {
            return None;
        }
    }
    if !f_dst.join("package.json").exists() {
        let _ = fs::remove_dir_all(&f_dst);
        if copy_dir_recursive(&f_src, &f_dst).is_err() {
            return None;
        }
    }
    if b_dst.join("package.json").exists() && f_dst.join("package.json").exists() {
        Some((canon(&b_dst), canon(&f_dst)))
    } else {
        None
    }
}

/// Gabungan penemuan relatif (sibling/cwd/config) dengan fallback ter-bundle.
fn resolve_project_dirs(app: &tauri::AppHandle) -> DiscoveryResult {
    let mut res = find_project_dirs();
    if res.backend.is_none() || res.frontend.is_none() {
        if let Some((b, f)) = extract_bundled(app) {
            if res.backend.is_none() {
                res.backend = Some(b);
            }
            if res.frontend.is_none() {
                res.frontend = Some(f);
            }
        }
    }
    res
}

// ───────────────────────── Process helpers ─────────────────────────

/// Program pm2: coba `pm2` di PATH, jika tidak ada pakai `bun x pm2`.
fn pm2_program() -> Vec<String> {
    if let Some(p) = resolve_bin("pm2") {
        vec![p]
    } else {
        vec![
            resolve_bin("bun").unwrap_or_else(|| "bun".to_string()),
            "x".to_string(),
            "pm2".to_string(),
        ]
    }
}

/// Jalankan perintah & stream stdout+stderr ke event UI secara real-time.
fn run_cmd_log(
    app: &tauri::AppHandle,
    parts: &[&str],
    cwd: Option<&str>,
) -> Result<(), String> {
    let prog = resolve_bin(parts[0]).unwrap_or_else(|| parts[0].to_string());
    let mut cmd = Command::new(&prog);
    cmd.args(&parts[1..]);
    if let Some(c) = cwd {
        cmd.current_dir(c);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Gagal menjalankan '{}': {}", parts[0], e))?;

    let app2 = app.clone();
    if let Some(err) = child.stderr.take() {
        std::thread::spawn(move || {
            for line in BufReader::new(err).lines().flatten() {
                let _ = app2.emit("install-log", format!("[stderr] {}", line));
            }
        });
    }
    if let Some(out) = child.stdout.take() {
        for line in BufReader::new(out).lines().flatten() {
            let _ = app.emit("install-log", line);
        }
    }

    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!(
            "Perintah gagal (exit {}): {}",
            status.code().unwrap_or(-1),
            parts.join(" ")
        ));
    }
    Ok(())
}

/// Jalankan perintah pm2 (tanpa streaming log ke UI).
fn run_pm2(args: &[&str], cwd: Option<&str>) -> Result<(), String> {
    let prog = pm2_program();
    let mut cmd = Command::new(&prog[0]);
    cmd.args(&prog[1..]);
    cmd.args(args);
    if let Some(c) = cwd {
        cmd.current_dir(c);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Gagal menjalankan pm2: {}", e))?;

    let mut out = String::new();
    if let Some(o) = child.stdout.take() {
        for line in BufReader::new(o).lines().flatten() {
            out.push_str(&line);
            out.push('\n');
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("pm2 gagal ({}):\n{}", args.join(" "), out));
    }
    Ok(())
}

/// Cari path absolut sebuah executable (bun/pm2) agar deteksi & eksekusi
/// tetap bekerja meski PATH tidak diwariskan utuh ke child process
/// (umum di macOS GUI / app yang di-klik).
fn resolve_bin(prog: &str) -> Option<String> {
    // 1. Coba via PATH (pakai login shell supaya profile PATH kebaca di macOS).
    #[cfg(windows)]
    let out = Command::new("cmd").args(["/C", &format!("where {}", prog)]).output();
    #[cfg(not(windows))]
    let out = Command::new("sh")
        .args(["-lc", &format!("command -v {}", prog)])
        .output();

    if let Ok(o) = out {
        if o.status.success() {
            if let Ok(s) = String::from_utf8(o.stdout) {
                let first = s.lines().next().unwrap_or("").trim().to_string();
                if !first.is_empty() {
                    return Some(first);
                }
            }
        }
    }

    // 2. Lokasi pemasangan umum.
    let mut candidates: Vec<String> = Vec::new();
    if let Some(home) = std::env::var("HOME").ok() {
        // nvm (bun/pm2 sering terpasang lewat `npm i -g` di sini)
        let nvm_base = format!("{}/.nvm/versions/node", home);
        if let Ok(entries) = std::fs::read_dir(&nvm_base) {
            for e in entries.flatten() {
                let ver = e.file_name();
                candidates.push(format!(
                    "{}/{}/bin/{}",
                    nvm_base,
                    ver.to_string_lossy(),
                    prog
                ));
            }
        }
        candidates.push(format!("{}/.bun/bin/{}", home, prog));
        candidates.push(format!("{}/.cargo/bin/{}", home, prog));
    }
    if cfg!(windows) {
        if let Some(up) = std::env::var("USERPROFILE").ok() {
            candidates.push(format!("{}\\.bun\\bin\\{}", up, prog));
        }
    } else {
        candidates.push(format!("/usr/local/bin/{}", prog));
        candidates.push(format!("/opt/homebrew/bin/{}", prog));
        candidates.push(format!("/usr/bin/{}", prog));
    }

    for c in &candidates {
        if Path::new(c).exists() {
            return Some(c.clone());
        }
        #[cfg(windows)]
        {
            let exe = format!("{}.exe", c);
            if Path::new(&exe).exists() {
                return Some(exe);
            }
        }
    }
    None
}

fn run_check(prog: &str) -> bool {
    resolve_bin(prog).is_some()
}

// ───────────────────────── Tauri Commands ─────────────────────────

#[tauri::command]
fn discover(app: tauri::AppHandle) -> DiscoveryResult {
    resolve_project_dirs(&app)
}

#[tauri::command(rename = "check-system-status")]
fn check_system_status(backend: String, frontend: String) -> StatusResult {
    let pg_installed = check_postgres_installed();
    let pg_running = check_postgres_running();
    let pm2 = run_check("pm2");
    let deps = Path::new(&backend).join("node_modules").exists()
        && Path::new(&frontend).join("node_modules").exists();
    let built = Path::new(&frontend).join("build").exists()
        || Path::new(&frontend).join("dist").exists();

    StatusResult {
        is_postgres_installed: pg_installed,
        is_postgres_running: pg_running,
        is_pm2_installed: pm2,
        are_deps_installed: deps,
        is_frontend_built: built,
        backend_path: if backend.is_empty() { None } else { Some(backend) },
        frontend_path: if frontend.is_empty() { None } else { Some(frontend) },
    }
}

/// Pasang Bun secara otomatis bila belum ada (bootstrap installer).
fn install_bun(app: &tauri::AppHandle) -> Result<(), String> {
    #[cfg(windows)]
    {
        run_cmd_log(
            app,
            &[
                "powershell",
                "-NoProfile",
                "-ExecutionPolicy",
                "ByPass",
                "-Command",
                "irm https://bun.sh/install.ps1 | iex",
            ],
            None,
        )
    }
    #[cfg(not(windows))]
    {
        run_cmd_log(app, &["sh", "-c", "curl -fsSL https://bun.sh/install | bash"], None)
    }
}

#[tauri::command(rename = "run-installer")]
fn run_installer(app: tauri::AppHandle, backend: String, frontend: String) -> Result<(), String> {
    let _ = app.emit("install-log", "▶ Memulai instalasi CBT...");

    // Bootstrap: pasang Bun dulu bila belum terdeteksi.
    if !run_check("bun") {
        let _ = app.emit("install-log", "▶ Bun belum ada. Menginstal Bun...");
        install_bun(&app)?;
        if !run_check("bun") {
            return Err(
                "Gagal memasang Bun. Periksa koneksi internet dan coba lagi.".into(),
            );
        }
        let _ = app.emit("install-log", "✔ Bun terpasang.");
    } else {
        let _ = app.emit("install-log", "✔ Bun sudah terpasang.");
    }

    run_cmd_log(&app, &["bun", "add", "-g", "pm2"], None)?;
    let _ = app.emit("install-log", "▶ Install dependensi backend...");
    run_cmd_log(&app, &["bun", "install"], Some(&backend))?;
    let _ = app.emit("install-log", "▶ Install dependensi frontend...");
    run_cmd_log(&app, &["bun", "install"], Some(&frontend))?;
    let _ = app.emit("install-log", "▶ Build frontend...");
    run_cmd_log(&app, &["bun", "run", "build"], Some(&frontend))?;
    let _ = app.emit("install-log", "✔ Instalasi selesai.");
    Ok(())
}

#[tauri::command(rename = "start-services")]
fn start_services(app: tauri::AppHandle, backend: String, frontend: String) -> Result<(), String> {
    // Hapus proses lama (jika ada) agar start bersifat idempoten & tidak
    // bentrok di port yang sama.
    let _ = run_pm2(&["delete", "cbt-frontend", "cbt-backend"], None);

    // Jalankan proxy statis + reverse-proxy /api ke backend (menggantikan
    // `pm2 serve` yang hanya statis & tidak meneruskan API same-origin).
    let proxy = find_proxy(&app, &frontend).ok_or_else(|| {
        "proxy-server.mjs tidak ditemukan (pastikan file ada di folder launcher)".to_string()
    })?;
    run_pm2(
        &["start", &proxy, "--name", "cbt-frontend", "--interpreter", "bun"],
        None,
    )?;

    let be_index = Path::new(&backend)
        .join("src")
        .join("index.ts")
        .to_string_lossy()
        .to_string();
    run_pm2(
        &["start", &be_index, "--name", "cbt-backend", "--interpreter", "bun"],
        None,
    )?;
    Ok(())
}

#[tauri::command(rename = "stop-services")]
fn stop_services() -> Result<(), String> {
    // Diabaikan jika memang belum jalan.
    let _ = run_pm2(&["stop", "cbt-frontend", "cbt-backend"], None);
    run_pm2(&["delete", "cbt-frontend", "cbt-backend"], None)?;
    Ok(())
}

#[tauri::command(rename = "check-running")]
fn check_running() -> RunningStatus {
    let mut fe = false;
    let mut be = false;
    let prog = pm2_program();
    let mut cmd = Command::new(&prog[0]);
    cmd.args(&prog[1..]).arg("jlist");
    cmd.stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    if let Ok(out) = cmd.output() {
        if out.status.success() {
            if let Ok(s) = String::from_utf8(out.stdout) {
                if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(&s) {
                    for p in arr {
                        let name = p.get("name").and_then(|n| n.as_str()).unwrap_or("");
                        let st = p
                            .get("pm2_env")
                            .and_then(|e| e.get("status"))
                            .and_then(|x| x.as_str())
                            .unwrap_or("");
                        if name == "cbt-frontend" && st == "online" {
                            fe = true;
                        }
                        if name == "cbt-backend" && st == "online" {
                            be = true;
                        }
                    }
                }
            }
        }
    }
    RunningStatus {
        frontend: fe,
        backend: be,
    }
}

// ───────────────────────── PostgreSQL ─────────────────────────

#[allow(dead_code)]
struct DbCreds {
    user: String,
    password: String,
    host: String,
    port: u16,
    dbname: String,
}

/// Path binari PostgreSQL (postgres/initdb/pg_ctl/psql) bila terpasang.
fn pg_bin_dir() -> Option<PathBuf> {
    if let Some(p) = resolve_bin("psql") {
        if let Some(parent) = Path::new(&p).parent() {
            return Some(parent.to_path_buf());
        }
    }
    if let Some(p) = resolve_bin("postgres") {
        if let Some(parent) = Path::new(&p).parent() {
            return Some(parent.to_path_buf());
        }
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        if let Some(pf) = std::env::var("ProgramFiles").ok() {
            candidates.push(PathBuf::from(pf).join("PostgreSQL"));
        }
        candidates.push(PathBuf::from("C:\\Program Files\\PostgreSQL"));
    } else if cfg!(target_os = "macos") {
        candidates.push(PathBuf::from("/opt/homebrew/opt/postgresql@16/bin"));
        candidates.push(PathBuf::from("/opt/homebrew/opt/postgresql/bin"));
        candidates.push(PathBuf::from("/usr/local/opt/postgresql@16/bin"));
        candidates.push(PathBuf::from("/usr/local/opt/postgresql/bin"));
    } else {
        candidates.push(PathBuf::from("/usr/lib/postgresql/16/bin"));
        candidates.push(PathBuf::from("/usr/lib/postgresql/15/bin"));
        candidates.push(PathBuf::from("/usr/lib/postgresql/14/bin"));
    }
    for base in &candidates {
        if !base.exists() {
            continue;
        }
        if cfg!(windows) {
            if let Ok(entries) = fs::read_dir(base) {
                for e in entries.flatten() {
                    let bin = e.path().join("bin");
                    if bin.join("postgres.exe").exists() {
                        return Some(bin);
                    }
                }
            }
        } else if base.join("postgres").exists() || base.join("psql").exists() {
            return Some(base.clone());
        }
    }
    None
}

fn bin_exe(dir: &Path, name: &str) -> PathBuf {
    if cfg!(windows) {
        dir.join(format!("{}.exe", name))
    } else {
        dir.join(name)
    }
}

fn check_postgres_installed() -> bool {
    pg_bin_dir().is_some()
}

fn check_postgres_running() -> bool {
    match TcpStream::connect_timeout(
        &SocketAddr::from(([127u8, 0, 0, 1], 5432)),
        Duration::from_millis(800),
    ) {
        Ok(_) => true,
        Err(_) => false,
    }
}

fn random_password() -> String {
    let mut seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9e37_79b9_7f4a_7c15);
    let charset: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut s = String::with_capacity(24);
    for _ in 0..24 {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let idx = (seed >> 33) as usize % charset.len();
        s.push(charset[idx] as char);
    }
    s
}

/// Parse `postgres://user:pass@host:port/dbname` menjadi kredensial.
fn parse_db_creds(url: &str) -> Option<DbCreds> {
    let s = url.split("://").nth(1)?;
    let (auth, rest) = s.split_once('@')?;
    let (user, password) = match auth.split_once(':') {
        Some((u, p)) => (u.to_string(), p.to_string()),
        None => (auth.to_string(), String::new()),
    };
    let (hostport, dbname) = match rest.split_once('/') {
        Some((hp, db)) => (hp, db),
        None => (rest, ""),
    };
    let (host, port) = match hostport.split_once(':') {
        Some((h, p)) => (h.to_string(), p.parse::<u16>().unwrap_or(5432)),
        None => (hostport.to_string(), 5432),
    };
    if user.is_empty() {
        return None;
    }
    Some(DbCreds {
        user,
        password,
        host,
        port,
        dbname: dbname.to_string(),
    })
}

/// Baca/tulis `backend/.env`: pastikan DATABASE_URL (lokal) & JWT_SECRET ada.
fn ensure_backend_env(backend: &str) -> Result<DbCreds, String> {
    let env_path = Path::new(backend).join(".env");
    let mut lines: Vec<String> = if let Ok(s) = fs::read_to_string(&env_path) {
        s.lines().map(|l| l.to_string()).collect()
    } else {
        Vec::new()
    };

    let mut creds = None;
    for l in &lines {
        if let Some(rest) = l.strip_prefix("DATABASE_URL=") {
            if let Some(c) = parse_db_creds(rest.trim()) {
                creds = Some(c);
                break;
            }
        }
    }

    if creds.is_none() {
        let user = "cbt_user".to_string();
        let password = random_password();
        let dbname = "cbt_db".to_string();
        let url = format!("postgres://{}:{}@127.0.0.1:5432/{}", user, password, dbname);
        creds = Some(DbCreds {
            user,
            password,
            host: "127.0.0.1".into(),
            port: 5432,
            dbname,
        });
        let mut found = false;
        for l in lines.iter_mut() {
            if l.starts_with("DATABASE_URL=") {
                *l = format!("DATABASE_URL={}", url);
                found = true;
                break;
            }
        }
        if !found {
            lines.push(format!("DATABASE_URL={}", url));
        }
    }

    let mut has_jwt = false;
    for l in &lines {
        if l.starts_with("JWT_SECRET=") {
            has_jwt = true;
            break;
        }
    }
    if !has_jwt {
        lines.push(format!("JWT_SECRET={}", random_password()));
    }

    fs::write(&env_path, lines.join("\n")).map_err(|e| e.to_string())?;
    creds.ok_or_else(|| "Gagal menyusun kredensial database.".into())
}

/// Pasang PostgreSQL via package manager sistem (winget/brew/apt).
fn install_postgres(app: &tauri::AppHandle) -> Result<(), String> {
    #[cfg(windows)]
    {
        run_cmd_log(
            app,
            &[
                "winget",
                "install",
                "-e",
                "--accept-package-agreements",
                "--accept-source-agreements",
                "PostgreSQL.PostgreSQL",
            ],
            None,
        )
        .map_err(|e| {
            format!(
                "Gagal memasang PostgreSQL via winget: {e}\nCoba instal manual dari https://www.postgresql.org/download/windows/"
            )
        })
    }
    #[cfg(target_os = "macos")]
    {
        run_cmd_log(
            app,
            &[
                "sh",
                "-c",
                "command -v brew >/dev/null 2>&1 || (curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh | bash)",
            ],
            None,
        )
        .ok();
        run_cmd_log(app, &["sh", "-c", "brew install postgresql@16"], None).map_err(|e| {
            format!(
                "Gagal memasang PostgreSQL via brew: {e}\nCoba instal manual: https://www.postgresql.org/download/macos/"
            )
        })
    }
    #[cfg(target_os = "linux")]
    {
        run_cmd_log(
            app,
            &["sh", "-c", "sudo apt-get update && sudo apt-get install -y postgresql"],
            None,
        )
        .map_err(|e| {
            format!(
                "Gagal memasang PostgreSQL via apt: {e}\nCoba instal manual: https://www.postgresql.org/download/linux/"
            )
        })
    }
}

/// Pasang + jalankan PostgreSQL lokal, siapkan role/db, lalu migrate & seed.
#[tauri::command(rename = "install-database")]
fn install_database(app: tauri::AppHandle, backend: String) -> Result<(), String> {
    let _ = app.emit("install-log", "▶ Memeriksa PostgreSQL...");
    if !check_postgres_installed() {
        let _ = app.emit("install-log", "▶ PostgreSQL belum ada. Menginstal...");
        install_postgres(&app)?;
        if !check_postgres_installed() {
            return Err(
                "PostgreSQL masih belum terdeteksi setelah instalasi. Instal manual lalu coba lagi."
                    .into(),
            );
        }
        let _ = app.emit("install-log", "✔ PostgreSQL terpasang.");
    } else {
        let _ = app.emit("install-log", "✔ PostgreSQL sudah terpasang.");
    }

    if !run_check("bun") {
        return Err("Bun belum terpasang. Jalankan INSTALL LAUNCHER terlebih dahulu.".into());
    }

    let bindir = pg_bin_dir().ok_or_else(|| "Binari PostgreSQL tidak ditemukan.".to_string())?;
    let initdb = bin_exe(&bindir, "initdb");
    let pg_ctl = bin_exe(&bindir, "pg_ctl");
    let psql = bin_exe(&bindir, "psql");

    let app_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let pgdata = app_dir.join("pgdata");
    fs::create_dir_all(&pgdata).map_err(|e| e.to_string())?;

    if !pgdata.join("PG_VERSION").exists() {
        let _ = app.emit("install-log", "▶ Inisialisasi cluster database...");
        let pwfile = pgdata.join("superpw.txt");
        fs::write(&pwfile, b"cbt-super-placeholder").map_err(|e| e.to_string())?;
        run_cmd_log(
            &app,
            &[
                &initdb.to_string_lossy(),
                "-D",
                &pgdata.to_string_lossy(),
                "-U",
                "postgres",
                "--pwfile",
                &pwfile.to_string_lossy(),
                "-A",
                "trust",
                "-E",
                "UTF8",
            ],
            None,
        )?;
        let hba = pgdata.join("pg_hba.conf");
        if let Ok(mut f) = fs::OpenOptions::new().append(true).open(&hba) {
            let _ = writeln!(f, "host all all 127.0.0.1/32 trust");
            let _ = writeln!(f, "host all all ::1/128 trust");
        }
        let _ = fs::remove_file(&pwfile);
    }

    if !check_postgres_running() {
        let _ = app.emit("install-log", "▶ Menjalankan PostgreSQL...");
        let logfile = pgdata.join("logfile");
        let mut opts = String::from("-p 5432 -c listen_addresses=127.0.0.1");
        if !cfg!(windows) {
            opts.push_str(&format!(
                " -c unix_socket_directories={}",
                pgdata.to_string_lossy()
            ));
        }
        run_cmd_log(
            &app,
            &[
                &pg_ctl.to_string_lossy(),
                "-D",
                &pgdata.to_string_lossy(),
                "-l",
                &logfile.to_string_lossy(),
                "-o",
                &opts,
                "start",
            ],
            None,
        )?;
        let mut ready = false;
        for _ in 0..60 {
            if check_postgres_running() {
                ready = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        if !ready {
            return Err("PostgreSQL tidak mau start. Periksa log di direktori data.".into());
        }
        let _ = app.emit("install-log", "✔ PostgreSQL berjalan.");
    } else {
        let _ = app.emit("install-log", "✔ PostgreSQL sudah berjalan.");
    }

    let creds = ensure_backend_env(&backend)?;
    let _ = app.emit(
        "install-log",
        &format!("▶ Menyiapkan role/db '{}'...", creds.dbname),
    );

    let create_role = format!(
        "DO $$ BEGIN IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname='{user}') THEN CREATE ROLE {user} LOGIN PASSWORD '{pw}'; END IF; END $$;",
        user = creds.user,
        pw = creds.password
    );
    run_cmd_log(
        &app,
        &[
            &psql.to_string_lossy(),
            "-h",
            "127.0.0.1",
            "-U",
            "postgres",
            "-c",
            &create_role,
        ],
        None,
    )?;

    let create_db = format!(
        "SELECT 'CREATE DATABASE {db} OWNER {user}' WHERE NOT EXISTS (SELECT FROM pg_database WHERE datname='{db}')\\gexec",
        db = creds.dbname,
        user = creds.user
    );
    run_cmd_log(
        &app,
        &[
            &psql.to_string_lossy(),
            "-h",
            "127.0.0.1",
            "-U",
            "postgres",
            "-c",
            &create_db,
        ],
        None,
    )?;

    let _ = app.emit("install-log", "▶ Migrasi skema database...");
    run_cmd_log(&app, &["bun", "run", "db:migrate"], Some(&backend))?;
    let _ = app.emit("install-log", "▶ Seed data awal...");
    run_cmd_log(&app, &["bun", "run", "db:seed"], Some(&backend))?;

    let _ = app.emit("install-log", "✔ Database siap.");
    Ok(())
}

// ───────────────────────── Entry point ─────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            discover,
            check_system_status,
            run_installer,
            install_database,
            start_services,
            stop_services,
            check_running
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
