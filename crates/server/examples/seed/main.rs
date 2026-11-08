//! Development seed: fills a LOCAL cloudit server with demo data through its real API
//! (the sync protocol of `cloudit-core`), so the web UI has something to show.
//!
//! ```text
//! cargo run -p cloudit-server --example seed -- [--url http://127.0.0.1:8080]
//!     [--db <path to cloudit.db>] [--setup-token <token>]
//! ```
//!
//! (`make seed`, with `SEED_ARGS` for the options.) It refuses any host that is not
//! the local machine. On a fresh server it runs the first-run setup (the token comes
//! from `--setup-token` or the `SETUP_TOKEN` environment variable); it creates three
//! users (an admin and two friends, through invites), scopes shared with different
//! roles, folders with emoji, several versions over different days, a pinned and
//! labelled version, a restore and an unresolved conflict.
//!
//! The API cannot set creation times, so with `--db` (the SQLite file the server uses)
//! the tool back-dates the snapshots there; without it every version is "today".
//! It stops if the admin already has scopes: start from an empty data directory.

mod api;
mod data;

use anyhow::{Context as _, Result, bail};
use api::Api;
use chrono::{Duration, Utc};
use reqwest::Method;
use sea_orm::sea_query::Value as DbValue;
use sea_orm::{ConnectionTrait as _, Database, DbBackend, Statement};
use serde_json::{Value, json};

const PASSWORD: &str = "demo-password-1";
const ADMIN: (&str, &str) = ("jacopo@demo.test", "Jacopo");
const MARCO: (&str, &str) = ("marco@demo.test", "Marco");
const LUCA: (&str, &str) = ("luca@demo.test", "Luca");

struct Options {
    url: String,
    db: Option<String>,
    setup_token: Option<String>,
}

fn options() -> Result<Options> {
    let mut options = Options {
        url: "http://127.0.0.1:8080".to_owned(),
        db: None,
        setup_token: std::env::var("SETUP_TOKEN").ok().filter(|t| !t.is_empty()),
    };
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .with_context(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--url" => options.url = value,
            "--db" => options.db = Some(value),
            "--setup-token" => options.setup_token = Some(value),
            other => bail!("unknown option {other}"),
        }
    }
    let url = url::Url::parse(&options.url).context("--url")?;
    let local = match url.host() {
        Some(url::Host::Domain(name)) => name == "localhost",
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    };
    if !local {
        bail!(
            "refusing to seed {}: only a local server is allowed",
            options.url
        );
    }
    Ok(options)
}

/// Snapshots to back-date: (id, age).
type Ages = Vec<(String, Duration)>;

fn id(value: &Value) -> Result<String> {
    value["id"]
        .as_str()
        .map(str::to_owned)
        .context("no id in the answer")
}

async fn folder(api: &Api, scope: &str, name: &str, emoji: &str) -> Result<String> {
    id(&api
        .post(
            &format!("/scopes/{scope}/folders"),
            json!({ "name": name, "emoji": emoji }),
        )
        .await?)
}

#[tokio::main]
async fn main() -> Result<()> {
    let options = options()?;
    let anon = Api::new(&options.url)?;

    // The admin: first-run setup, or sign in when the server is already set up.
    let setup = anon.get("/setup").await?;
    if setup["required"] == true {
        let token = options
            .setup_token
            .as_deref()
            .context("first-run setup needs --setup-token or SETUP_TOKEN")?;
        let answer = anon
            .json(
                Method::POST,
                "/setup",
                Some(json!({
                    "token": token, "email": ADMIN.0, "password": PASSWORD,
                    "display_name": ADMIN.1, "registration_mode": "invite",
                })),
            )
            .await;
        answer.context("setup")?;
    }
    let mut jacopo = anon.clone();
    jacopo
        .login(ADMIN.0, PASSWORD)
        .await
        .context("signing in as the admin")?;
    if !jacopo
        .get("/scopes")
        .await?
        .as_array()
        .is_some_and(Vec::is_empty)
    {
        bail!("the admin already has scopes: start from an empty data directory");
    }

    // The friends sign up with invites (which also approves them).
    let mut people = Vec::new();
    for (email, name) in [MARCO, LUCA] {
        let invite = jacopo
            .post("/admin/invites", json!({ "email": email }))
            .await?;
        anon.post(
            "/auth/register",
            json!({
                "email": email, "password": PASSWORD, "display_name": name,
                "invite_token": invite["token"],
            }),
        )
        .await?;
        let mut api = anon.clone();
        api.login(email, PASSWORD).await?;
        people.push(api);
    }
    let marco = &people[0];
    let pc = jacopo.device("PC-Fisso", "windows").await?;
    let laptop = jacopo.device("Laptop", "linux").await?;
    let marco_laptop = marco.device("Laptop-Marco", "windows").await?;

    let mut ages: Ages = Vec::new();
    let hours = Duration::hours;
    let days = Duration::days;
    let mins = Duration::minutes;

    // minecraft: owned by Jacopo, shared with Marco (editor) and Luca (viewer).
    let minecraft = id(&jacopo
        .post("/scopes", json!({ "name": "minecraft" }))
        .await?)?;
    for (email, role) in [(MARCO.0, "editor"), (LUCA.0, "viewer")] {
        jacopo
            .post(
                &format!("/scopes/{minecraft}/members"),
                json!({ "email": email, "role": role }),
            )
            .await?;
    }

    // mondo1: eight versions over ten days, one pinned, one restore.
    let mondo = folder(&jacopo, &minecraft, "mondo1", "🌍").await?;
    let mut head: Option<String> = None;
    let steps: [(&Api, u64, Duration); 5] = [
        (&pc, 0, days(10)),
        (&pc, 1, days(9)),
        (&marco_laptop, 2, days(3) + hours(2)),
        (&pc, 3, days(3)),
        (&pc, 4, days(1) + hours(1)),
    ];
    let mut pinned = String::new();
    for (device, generation, age) in steps {
        let snapshot = device
            .commit(&mondo, &data::world(generation), head.as_deref(), "normal")
            .await?;
        if generation == 2 {
            pinned.clone_from(&snapshot);
        }
        ages.push((snapshot.clone(), age));
        head = Some(snapshot);
    }
    jacopo
        .patch(
            &format!("/snapshots/{pinned}"),
            json!({ "pinned": true, "label": "Prima di Ender Dragon" }),
        )
        .await?;
    let restored = jacopo
        .post(
            &format!("/snapshots/{pinned}/restore"),
            json!({ "expected_head": head }),
        )
        .await?;
    let restored = id(&restored)?;
    ages.push((restored.clone(), days(1)));
    let after = pc
        .commit(&mondo, &data::world(5), Some(&restored), "normal")
        .await?;
    ages.push((after.clone(), hours(2)));
    let last = pc
        .commit(&mondo, &data::world(6), Some(&after), "normal")
        .await?;
    ages.push((last, mins(3)));

    // isola-coop: the laptop played offline and lost the race: an open conflict.
    let island = folder(&jacopo, &minecraft, "isola-coop", "🏝️").await?;
    let base = pc
        .commit(&island, &data::island(0, false), None, "normal")
        .await?;
    ages.push((base.clone(), days(4)));
    let server = pc
        .commit(&island, &data::island(1, false), Some(&base), "normal")
        .await?;
    ages.push((server, days(1) + hours(3)));
    let conflict = laptop
        .commit(&island, &data::island(0, true), Some(&base), "conflict")
        .await?;
    ages.push((conflict, hours(5)));

    // mod-config: two versions by Marco's laptop.
    let mods = folder(marco, &minecraft, "mod-config", "⚙️").await?;
    let first = marco_laptop
        .commit(&mods, &data::mods(), None, "normal")
        .await?;
    ages.push((first.clone(), days(6)));
    // ... and a conflict: Jacopo's laptop worked from the first version.
    let newer = marco_laptop
        .commit(&mods, &data::mods_v2(), Some(&first), "normal")
        .await?;
    ages.push((newer, days(5)));
    let clash = laptop
        .commit(&mods, &data::mods_v3(), Some(&first), "conflict")
        .await?;
    ages.push((clash, hours(3)));

    // documenti: Jacopo only.
    let documents = id(&jacopo
        .post("/scopes", json!({ "name": "documenti" }))
        .await?)?;
    let contracts = folder(&jacopo, &documents, "Contratti", "📄").await?;
    let one = pc
        .commit(&contracts, &data::documents(0), None, "normal")
        .await?;
    ages.push((one.clone(), days(20)));
    let two = pc
        .commit(&contracts, &data::documents(1), Some(&one), "normal")
        .await?;
    ages.push((two, days(2)));

    // foto-vacanze: Marco's, with Jacopo as an admin member.
    let photos = id(&marco
        .post("/scopes", json!({ "name": "foto-vacanze" }))
        .await?)?;
    marco
        .post(
            &format!("/scopes/{photos}/members"),
            json!({ "email": ADMIN.0, "role": "admin" }),
        )
        .await?;
    let summer = folder(marco, &photos, "Estate 2025", "📷").await?;
    let shot = marco_laptop
        .commit(&summer, &data::photos(), None, "normal")
        .await?;
    ages.push((shot, days(30)));

    backdate(options.db.as_deref(), &ages).await?;

    println!("Seeded {} with demo data.", options.url);
    println!("Sign in with password '{PASSWORD}' as:");
    println!("  {}  (admin, owns minecraft and documenti)", ADMIN.0);
    println!("  {}  (editor in minecraft, owns foto-vacanze)", MARCO.0);
    println!("  {}  (viewer in minecraft)", LUCA.0);
    Ok(())
}

/// Moves the snapshots' creation times into the past in the server's SQLite file.
async fn backdate(db: Option<&str>, ages: &Ages) -> Result<()> {
    let Some(path) = db else {
        println!("No --db given: every version is dated today.");
        return Ok(());
    };
    let connection = Database::connect(format!("sqlite:{path}?mode=rw"))
        .await
        .with_context(|| format!("opening {path}"))?;
    let now = Utc::now();
    for (snapshot, age) in ages {
        let id: uuid::Uuid = snapshot.parse().context("snapshot id")?;
        connection
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "UPDATE snapshots SET created_at = ? WHERE id = ?",
                [DbValue::from(now - *age), DbValue::from(id)],
            ))
            .await
            .context("back-dating a snapshot")?;
    }
    Ok(())
}
