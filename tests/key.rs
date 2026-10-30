//! `kkt key create`: a new key in a file only its owner can read, never
//! printed unless asked, and never written over by accident.

#[path = "common/home.rs"]
mod home;
#[path = "common/scratch.rs"]
mod scratch;
use home::OwnHome;

use std::path::Path;
use std::process::Output;
use std::time::Duration;
use tokio::process::Command;

const KKT: &str = env!("CARGO_BIN_EXE_kkt");
const TIMEOUT: Duration = Duration::from_secs(20);

/// An empty home, removed when dropped; a test declares it before the `kkt`
/// processes that use it.
fn temp_home(name: &str) -> scratch::Dir {
    scratch::Dir::new(&format!("kashkeshet-key-{name}"))
}

async fn kkt(home: &Path, args: &[&str]) -> Output {
    let run = Command::new(KKT)
        .args(args)
        .own_home(home)
        .kill_on_drop(true)
        .output();
    tokio::time::timeout(TIMEOUT, run)
        .await
        .expect("kkt key create should finish")
        .expect("run kkt")
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn read_key(path: &Path) -> String {
    std::fs::read_to_string(path)
        .expect("read key")
        .trim()
        .to_string()
}

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777
}

#[tokio::test]
async fn creates_a_private_key_file_and_does_not_print_the_key() {
    let home = temp_home("default");
    let out = kkt(&home, &["key", "create"]).await;
    assert!(out.status.success(), "{}", text(&out));
    let path = home.join(".config/kkt/access-key");
    let key = read_key(&path);
    assert_eq!(key.len(), 50);
    let printed = text(&out);
    assert!(printed.contains(&path.display().to_string()), "{printed}");
    assert!(printed.contains("--access-key-file"), "{printed}");
    assert!(!printed.contains(&key), "the key was printed: {printed}");
    #[cfg(unix)]
    assert_eq!(mode(&path), 0o600);
}

#[tokio::test]
async fn refuses_to_overwrite_without_force() {
    let home = temp_home("overwrite");
    let path = home.join("my-key");
    let path_arg = path.display().to_string();
    let first = kkt(&home, &["key", "create", "--path", &path_arg]).await;
    assert!(first.status.success(), "{}", text(&first));
    let key = read_key(&path);

    let again = kkt(&home, &["key", "create", "--path", &path_arg]).await;
    assert!(!again.status.success());
    assert!(
        text(&again).contains("already exists; pass --force"),
        "{}",
        text(&again)
    );
    assert_eq!(read_key(&path), key, "the key was replaced without --force");

    // A replaced file is private again, even if it was not before.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");
    }
    let forced = kkt(&home, &["key", "create", "--path", &path_arg, "--force"]).await;
    assert!(forced.status.success(), "{}", text(&forced));
    assert_ne!(read_key(&path), key);
    #[cfg(unix)]
    assert_eq!(mode(&path), 0o600);
}

#[tokio::test]
async fn show_prints_the_key() {
    let home = temp_home("show");
    let path = home.join("shown-key");
    let path_arg = path.display().to_string();
    let out = kkt(&home, &["key", "create", "--path", &path_arg, "--show"]).await;
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains(&read_key(&path)));
}

/// The key `kkt key create` writes is one the server accepts.
#[tokio::test]
async fn the_server_accepts_a_created_key() {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let home = temp_home("accepted");
    let out = kkt(&home, &["key", "create"]).await;
    assert!(out.status.success(), "{}", text(&out));
    let key_file = home.join(".config/kkt/access-key");
    let key = read_key(&key_file);
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("free port")
        .local_addr()
        .expect("addr")
        .port();
    let bind = format!("127.0.0.1:{port}");
    let mut child = Command::new(KKT)
        .args(["serve", "--bind", &bind, "--access-key-file"])
        .arg(&key_file)
        .own_home(&home)
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("start kkt");
    let mut lines = BufReader::new(child.stdout.take().expect("stdout")).lines();
    tokio::time::timeout(TIMEOUT, async {
        while let Some(line) = lines.next_line().await.expect("read stdout") {
            if line.contains("listening") {
                return;
            }
        }
        panic!("kkt exited before listening");
    })
    .await
    .expect("kkt did not start in time");

    let http = reqwest::Client::new();
    let status = |auth: Option<String>| {
        let mut request = http.post(format!("http://{bind}/mcp"));
        if let Some(auth) = auth {
            request = request.header("Authorization", auth);
        }
        async move { request.send().await.expect("post /mcp").status().as_u16() }
    };
    assert_eq!(status(None).await, 401);
    assert_ne!(status(Some(format!("Bearer {key}"))).await, 401);

    child.kill().await.expect("stop kkt");
}

#[tokio::test]
async fn show_prints_only_the_key() {
    let home = temp_home("show-only");
    let path = home.join("key");
    let key = "k".repeat(40);
    std::fs::write(&path, format!("{key}\n")).expect("write key");
    let out = kkt(
        &home,
        &["key", "show", "--key-file", path.to_str().expect("path")],
    )
    .await;
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(String::from_utf8_lossy(&out.stdout), format!("{key}\n"));
    assert!(out.stderr.is_empty());
}

#[tokio::test]
async fn show_without_a_key_file_is_an_error() {
    let home = temp_home("show-missing");
    let out = kkt(&home, &["key", "show"]).await;
    assert!(!out.status.success());
    assert!(
        text(&out).contains("could not read key file"),
        "{}",
        text(&out)
    );
}

#[tokio::test]
async fn web_key_creation_show_and_rotation_are_separate_from_access_key() {
    let home = temp_home("web");
    for args in [vec!["key", "create"], vec!["key", "create", "--web"]] {
        let out = kkt(&home, &args).await;
        assert!(out.status.success(), "{}", text(&out));
    }
    let access_file = home.join(".config/kkt/access-key");
    let web_file = home.join(".config/kkt/web-key");
    let access = read_key(&access_file);
    let web = read_key(&web_file);
    assert_ne!(access, web);
    #[cfg(unix)]
    assert_eq!(mode(&web_file), 0o600);
    let out = kkt(&home, &["key", "show", "--web"]).await;
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), web);
    let out = kkt(&home, &["key", "header"]).await;
    assert!(out.status.success());
    assert!(!text(&out).contains(&web));
    let out = kkt(&home, &["key", "create", "--web", "--force"]).await;
    assert!(out.status.success(), "{}", text(&out));
    assert!(!text(&out).contains(&read_key(&web_file)));
    assert_ne!(read_key(&web_file), web);
    assert_eq!(read_key(&access_file), access);
}

async fn start_server(home: &Path) -> (tokio::process::Child, String) {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let bind = listener.local_addr().unwrap().to_string();
    drop(listener);
    let mut child = Command::new(KKT)
        .args(["serve", "--bind", &bind])
        .env("HOME", home)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    tokio::time::timeout(TIMEOUT, async {
        while let Some(line) = lines.next_line().await.unwrap() {
            if line.contains("listening") {
                return;
            }
        }
        panic!("server exited before listening");
    })
    .await
    .unwrap();
    (child, format!("http://{bind}"))
}

#[tokio::test]
async fn rotating_the_web_key_and_restarting_ends_old_sessions() {
    let home = temp_home("web-sessions");
    let (mut server, base) = start_server(&home).await;
    let access = read_key(&home.join(".config/kkt/access-key"));
    let old_web = read_key(&home.join(".config/kkt/web-key"));
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    async fn login(http: &reqwest::Client, base: &str, key: &str) -> reqwest::Response {
        http.post(format!("{base}/login"))
            .header("origin", base)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(format!("key={key}"))
            .send()
            .await
            .unwrap()
    }
    let login_response = login(&http, &base, &old_web).await;
    let cookie = login_response.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    assert_eq!(
        http.get(format!("{base}/"))
            .header("cookie", &cookie)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let out = kkt(&home, &["key", "create", "--web", "--force"]).await;
    assert!(out.status.success(), "{}", text(&out));
    let new_web = read_key(&home.join(".config/kkt/web-key"));
    assert_ne!(old_web, new_web);
    server.kill().await.unwrap();
    let (mut server, base) = start_server(&home).await;
    assert_eq!(
        http.get(format!("{base}/"))
            .header("cookie", cookie)
            .send()
            .await
            .unwrap()
            .status(),
        303
    );
    assert_eq!(login(&http, &base, &old_web).await.status(), 401);
    assert_eq!(login(&http, &base, &access).await.status(), 401);
    assert!(
        login(&http, &base, &new_web)
            .await
            .headers()
            .contains_key("set-cookie")
    );
    assert_eq!(read_key(&home.join(".config/kkt/access-key")), access);
    server.kill().await.unwrap();
}

#[tokio::test]
async fn admin_key_creation_and_show_are_separate_from_the_other_keys() {
    let home = temp_home("admin");
    // No admin key has its own exit code, so setup can tell it from a key
    // it could not read.
    let out = kkt(&home, &["key", "show", "--admin"]).await;
    assert_eq!(out.status.code(), Some(4));
    assert!(out.stdout.is_empty());
    assert!(
        text(&out).contains("kkt key create --admin"),
        "{}",
        text(&out)
    );
    for args in [
        vec!["key", "create"],
        vec!["key", "create", "--web"],
        vec!["key", "create", "--admin"],
    ] {
        let out = kkt(&home, &args).await;
        assert!(out.status.success(), "{}", text(&out));
    }
    let config = home.join(".config/kkt");
    let (access, web) = (
        read_key(&config.join("access-key")),
        read_key(&config.join("web-key")),
    );
    let admin_file = config.join("admin-key");
    let admin = read_key(&admin_file);
    assert_ne!(admin, access);
    assert_ne!(admin, web);
    #[cfg(unix)]
    assert_eq!(mode(&admin_file), 0o600);
    let out = kkt(&home, &["key", "show", "--admin"]).await;
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), admin);
    // Never printed by create, and refused over an existing key.
    let again = kkt(&home, &["key", "create", "--admin"]).await;
    assert!(!again.status.success());
    let out = kkt(&home, &["key", "create", "--admin", "--force"]).await;
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("/mcp-admin"), "{}", text(&out));
    let rotated = read_key(&admin_file);
    assert!(!text(&out).contains(&rotated));
    assert_ne!(rotated, admin);
    assert_eq!(read_key(&config.join("access-key")), access);
    assert_eq!(read_key(&config.join("web-key")), web);
    let both = kkt(&home, &["key", "show", "--admin", "--web"]).await;
    assert!(!both.status.success());
}

#[tokio::test]
async fn an_admin_key_is_made_with_its_name_and_a_new_key_without_one_only_reads() {
    let home = temp_home("admin-name");
    let config = home.join(".config/kkt");
    let out = kkt(&home, &["key", "create"]).await;
    assert!(out.status.success(), "{}", text(&out));
    // A bad name leaves no key; --name needs --admin.
    for args in [
        vec!["key", "create", "--admin", "--name", "Night Owl"],
        vec!["key", "create", "--web", "--name", "sentinel"],
    ] {
        let out = kkt(&home, &args).await;
        assert!(!out.status.success(), "{args:?}");
    }
    assert!(!config.join("admin-key").exists());
    assert!(!config.join("admin-key.name").exists());

    let out = kkt(&home, &["key", "create", "--admin", "--name", "sentinel"]).await;
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("admin-sentinel"), "{}", text(&out));
    // The name, and a hash of the key it belongs to, never the key.
    let names = std::fs::read_to_string(config.join("admin-key.name")).unwrap();
    assert!(names.starts_with("admin-sentinel\nkey-sha256: "), "{names}");
    assert!(!names.contains(&read_key(&config.join("admin-key"))));
    // Standard output stays the key alone; the name is on standard error.
    let show = kkt(&home, &["key", "show", "--admin"]).await;
    assert!(show.status.success());
    assert_eq!(
        String::from_utf8_lossy(&show.stdout).trim(),
        read_key(&config.join("admin-key"))
    );
    assert!(
        String::from_utf8_lossy(&show.stderr).contains("admin name: admin-sentinel"),
        "{}",
        text(&show)
    );
    let status = text(&kkt(&home, &["status"]).await);
    assert!(status.contains("Admin name: admin-sentinel"), "{status}");

    // A new key made without a name has none: the old name went with the
    // old key.
    let out = kkt(&home, &["key", "create", "--admin", "--force"]).await;
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("may only read"), "{}", text(&out));
    assert!(!config.join("admin-key.name").exists());
    let show = kkt(&home, &["key", "show", "--admin"]).await;
    assert!(
        String::from_utf8_lossy(&show.stderr).contains("admin name: none"),
        "{}",
        text(&show)
    );
}

/// What `kkt key show --admin` says about the name of the admin key at
/// `path`, on standard error.
async fn shown_admin_name(home: &Path, path: &Path) -> String {
    let show = kkt(
        home,
        &[
            "key",
            "show",
            "--admin",
            "--key-file",
            path.to_str().unwrap(),
        ],
    )
    .await;
    assert!(show.status.success(), "{}", text(&show));
    String::from_utf8_lossy(&show.stderr).into_owned()
}

#[tokio::test]
async fn admin_keys_in_one_folder_keep_their_own_names() {
    let home = temp_home("admin-names-apart");
    let keys = home.join("keys");
    let (one, two, three) = (keys.join("one"), keys.join("two"), keys.join("three"));
    let create = |path: &Path, name: Option<&str>| {
        let mut args = vec!["key", "create", "--admin", "--path"];
        args.push(path.to_str().unwrap());
        if let Some(name) = name {
            args.extend(["--name", name]);
        }
        let home = home.to_path_buf();
        let args: Vec<String> = args.into_iter().map(String::from).collect();
        async move {
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let out = kkt(&home, &args).await;
            assert!(out.status.success(), "{}", text(&out));
        }
    };

    // A named key beside an unnamed one does not name it.
    create(&one, None).await;
    create(&two, Some("sentinel")).await;
    assert!(
        shown_admin_name(&home, &one)
            .await
            .contains("admin name: none"),
        "{}",
        shown_admin_name(&home, &one).await
    );
    assert!(
        shown_admin_name(&home, &two)
            .await
            .contains("admin name: admin-sentinel")
    );

    // An unnamed key beside a named one leaves its name alone.
    create(&three, None).await;
    assert!(
        shown_admin_name(&home, &two)
            .await
            .contains("admin name: admin-sentinel")
    );
    assert!(
        shown_admin_name(&home, &three)
            .await
            .contains("admin name: none")
    );
}

#[tokio::test]
async fn a_replaced_admin_key_does_not_keep_the_old_keys_name() {
    let home = temp_home("admin-name-rotated");
    let config = home.join(".config/kkt");
    let admin = config.join("admin-key");
    let out = kkt(&home, &["key", "create", "--admin", "--name", "sentinel"]).await;
    assert!(out.status.success(), "{}", text(&out));

    // Rotated with kkt and a new name: the new key has the new name.
    let out = kkt(
        &home,
        &["key", "create", "--admin", "--force", "--name", "night-owl"],
    )
    .await;
    assert!(out.status.success(), "{}", text(&out));
    let shown = shown_admin_name(&home, &admin).await;
    assert!(shown.contains("admin name: admin-night-owl"), "{shown}");

    // Replaced by hand with another key: the name file names nothing.
    let other = home.join("other-key");
    let out = kkt(&home, &["key", "create", "--path", other.to_str().unwrap()]).await;
    assert!(out.status.success(), "{}", text(&out));
    std::fs::write(&admin, std::fs::read_to_string(&other).unwrap()).unwrap();
    let shown = shown_admin_name(&home, &admin).await;
    assert!(shown.contains("admin name: none"), "{shown}");
    assert!(shown.contains("names a different key"), "{shown}");
    assert!(!shown.contains("admin-night-owl"), "{shown}");
    let status = text(&kkt(&home, &["status"]).await);
    assert!(status.contains("Admin name: none"), "{status}");
}

#[cfg(unix)]
#[tokio::test]
async fn a_name_that_cannot_be_written_leaves_the_new_admin_key_unnamed() {
    use std::os::unix::fs::PermissionsExt;
    let home = temp_home("admin-name-unwritable");
    let config = home.join(".config/kkt");
    let admin = config.join("admin-key");
    let names = config.join("admin-key.name");
    let out = kkt(&home, &["key", "create", "--admin", "--name", "sentinel"]).await;
    assert!(out.status.success(), "{}", text(&out));
    std::fs::set_permissions(&names, std::fs::Permissions::from_mode(0o444)).unwrap();
    if std::fs::OpenOptions::new().write(true).open(&names).is_ok() {
        // Run as root, where nothing stops the write.
        return;
    }

    let out = kkt(
        &home,
        &["key", "create", "--admin", "--force", "--name", "night-owl"],
    )
    .await;
    assert!(!out.status.success(), "{}", text(&out));
    assert!(
        text(&out).contains("its name could not be"),
        "{}",
        text(&out)
    );
    // The new key was written; the old key's name does not pass to it.
    let shown = shown_admin_name(&home, &admin).await;
    assert!(shown.contains("admin name: none"), "{shown}");
    assert!(!shown.contains("admin-sentinel"), "{shown}");
    assert!(!shown.contains("admin-night-owl"), "{shown}");
}

/// An admin key that is there but cannot be read is an error, not "no key".
#[tokio::test]
async fn an_admin_key_that_cannot_be_read_is_not_reported_as_missing() {
    let home = temp_home("admin-unreadable");
    let config = home.join(".config/kkt");
    std::fs::create_dir_all(&config).unwrap();
    // A folder where the key file should be cannot be read as a file, for
    // any user.
    std::fs::create_dir(config.join("admin-key")).unwrap();
    std::fs::write(config.join("access-key"), "a".repeat(40)).unwrap();
    let out = kkt(&home, &["key", "show", "--admin"]).await;
    assert!(!out.status.success());
    assert_ne!(out.status.code(), Some(4), "{}", text(&out));
    assert!(
        text(&out).contains("could not read key file"),
        "{}",
        text(&out)
    );
}

#[tokio::test]
async fn an_access_key_named_admin_key_still_serves_and_cannot_be_used_as_an_admin_key() {
    let home = temp_home("access-named-admin-key");
    let config = home.join(".config/kkt");
    std::fs::create_dir_all(&config).unwrap();
    let access_file = config.join("admin-key");
    let made = kkt(
        &home,
        &["key", "create", "--path", access_file.to_str().unwrap()],
    )
    .await;
    assert!(made.status.success(), "{}", text(&made));
    let access = read_key(&access_file);
    std::fs::write(
        config.join("config.toml"),
        "access_key_file = 'admin-key'\n",
    )
    .unwrap();

    let (mut server, base) = start_server(&home).await;
    let http = reqwest::Client::new();
    let unauthenticated = http.post(format!("{base}/mcp")).send().await.unwrap();
    assert_eq!(unauthenticated.status(), 401);
    let authenticated = http
        .post(format!("{base}/mcp"))
        .bearer_auth(&access)
        .send()
        .await
        .unwrap();
    assert_ne!(authenticated.status(), 401);
    for key in [None, Some(&access)] {
        let mut request = http.post(format!("{base}/mcp-admin"));
        if let Some(key) = key {
            request = request.bearer_auth(key);
        }
        assert_eq!(request.send().await.unwrap().status(), 404);
    }

    for args in [
        vec!["key", "create", "--admin"],
        vec!["key", "create", "--admin", "--force"],
        vec![
            "key",
            "create",
            "--admin",
            "--path",
            access_file.to_str().unwrap(),
        ],
        vec![
            "key",
            "create",
            "--admin",
            "--force",
            "--path",
            access_file.to_str().unwrap(),
        ],
    ] {
        let out = kkt(&home, &args).await;
        assert!(!out.status.success(), "{args:?}");
        let said = text(&out);
        assert!(
            said.contains("access_key_file is named admin-key"),
            "{said}"
        );
        assert!(said.contains("move one of the files"), "{said}");
        assert_eq!(read_key(&access_file), access);
    }
    for command in ["status", "doctor"] {
        let out = kkt(&home, &[command]).await;
        assert!(!text(&out).contains("dmin key"), "{}", text(&out));
    }
    assert!(!config.join("admin-key.name").exists());
    let elsewhere = home.join("other/admin-key");
    let out = kkt(
        &home,
        &[
            "key",
            "create",
            "--admin",
            "--path",
            elsewhere.to_str().unwrap(),
        ],
    )
    .await;
    assert!(out.status.success(), "{}", text(&out));
    assert_ne!(read_key(&elsewhere), access);
    assert_eq!(read_key(&access_file), access);
    server.kill().await.unwrap();
}

#[tokio::test]
async fn show_admin_reports_no_admin_key_when_the_access_key_is_named_admin_key() {
    let home = temp_home("show-access-named-admin-key");
    let config = home.join(".config/kkt");
    std::fs::create_dir_all(&config).unwrap();
    let access_file = config.join("admin-key");
    let made = kkt(
        &home,
        &["key", "create", "--path", access_file.to_str().unwrap()],
    )
    .await;
    assert!(made.status.success(), "{}", text(&made));
    let access = read_key(&access_file);
    std::fs::write(
        config.join("config.toml"),
        "access_key_file = 'admin-key'\n",
    )
    .unwrap();

    let out = kkt(&home, &["key", "show", "--admin"]).await;
    assert_eq!(out.status.code(), Some(4), "{}", text(&out));
    assert!(out.stdout.is_empty());
    assert!(
        text(&out).contains("there is no admin key"),
        "{}",
        text(&out)
    );
    assert!(!text(&out).contains("cannot add"), "{}", text(&out));
    assert_eq!(read_key(&access_file), access);
}

#[tokio::test]
async fn a_server_with_an_admin_key_serves_mcp_admin_with_it_alone() {
    let home = temp_home("admin-serve");
    let (mut server, base) = start_server(&home).await;
    let http = reqwest::Client::new();
    let status = |path: &'static str, key: Option<String>| {
        let mut request = http.post(format!("{base}{path}"));
        if let Some(key) = key {
            request = request.bearer_auth(key);
        }
        async move { request.send().await.unwrap().status().as_u16() }
    };
    // Off until an admin key exists.
    assert_eq!(status("/mcp-admin", None).await, 404);
    assert!(!home.join(".config/kkt/admin-key").exists());
    let out = kkt(&home, &["key", "create", "--admin"]).await;
    assert!(out.status.success(), "{}", text(&out));
    server.kill().await.unwrap();
    let (mut server, base) = start_server(&home).await;
    let status = |path: &'static str, key: Option<String>| {
        let mut request = http.post(format!("{base}{path}"));
        if let Some(key) = key {
            request = request.bearer_auth(key);
        }
        async move { request.send().await.unwrap().status().as_u16() }
    };
    let config = home.join(".config/kkt");
    let admin = read_key(&config.join("admin-key"));
    let access = read_key(&config.join("access-key"));
    let web = read_key(&config.join("web-key"));
    assert_ne!(status("/mcp-admin", Some(admin.clone())).await, 401);
    assert_eq!(status("/mcp-admin", Some(access.clone())).await, 401);
    assert_eq!(status("/mcp-admin", Some(web)).await, 401);
    assert_eq!(status("/mcp", Some(admin)).await, 401);
    assert_ne!(status("/mcp", Some(access)).await, 401);
    server.kill().await.unwrap();
}

#[tokio::test]
async fn status_and_doctor_name_the_admin_key_only_when_there_is_one() {
    let home = temp_home("admin-status");
    let out = kkt(&home, &["key", "create"]).await;
    assert!(out.status.success(), "{}", text(&out));
    for command in ["status", "doctor"] {
        let out = kkt(&home, &[command]).await;
        assert!(!text(&out).contains("dmin key"), "{}", text(&out));
    }
    let out = kkt(&home, &["key", "create", "--admin"]).await;
    assert!(out.status.success(), "{}", text(&out));
    let admin_file = home.join(".config/kkt/admin-key");
    let admin = read_key(&admin_file);
    let status = text(&kkt(&home, &["status"]).await);
    assert!(
        status.contains(&format!("Admin key: {}", admin_file.display())),
        "{status}"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&admin_file, std::fs::Permissions::from_mode(0o644)).unwrap();
        // No server runs here, so doctor finds problems; the admin key's
        // lines are among them.
        let doctor = text(&kkt(&home, &["doctor"]).await);
        assert!(
            doctor.contains("problem: admin key permissions: 0644, expected 0600"),
            "{doctor}"
        );
        assert!(doctor.contains("ok: admin key file"), "{doctor}");
        assert!(!doctor.contains(&admin), "{doctor}");
    }
    assert!(!status.contains(&admin), "{status}");
}

#[tokio::test]
async fn a_client_only_setup_cannot_show_or_create_an_admin_key() {
    let home = temp_home("admin-client");
    let config = home.join(".config/kkt");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("setup.toml"),
        "format = 1\nmode = \"client\"\nwritten_by = \"test\"\nurl = \"http://localhost:7420/mcp\"\n").unwrap();
    for action in ["show", "create"] {
        let out = kkt(&home, &["key", action, "--admin"]).await;
        assert!(!out.status.success());
        assert!(
            text(&out).contains("admin key is on the server's machine"),
            "{}",
            text(&out)
        );
    }
    assert!(!config.join("admin-key").exists());
}

#[tokio::test]
async fn a_client_only_setup_cannot_show_or_create_a_web_key() {
    let home = temp_home("web-client");
    let config = home.join(".config/kkt");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("setup.toml"),
        "format = 1\nmode = \"client\"\nwritten_by = \"test\"\nurl = \"http://localhost:7420/mcp\"\n").unwrap();
    for action in ["show", "create"] {
        let out = kkt(&home, &["key", action, "--web"]).await;
        assert!(!out.status.success());
        assert!(
            text(&out).contains("web key is on the server's machine"),
            "{}",
            text(&out)
        );
    }
    assert!(!config.join("web-key").exists());
}
