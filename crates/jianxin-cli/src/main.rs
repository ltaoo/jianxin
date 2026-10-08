//! `jianxin` — a multi-vendor email client for the terminal.
//!
//! Phase 1 ships the CLI first (§36): configure mailboxes in JSON, sync them
//! into a local SQLite database, read and send from the command line, and drop
//! into a TUI for the interactive case.

mod daemon;
mod output;
mod notifications;
mod watch;
mod web;

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::{CommandFactory, Parser, Subcommand};
use jianxin_config::{default_config_path, AppConfig};
use jianxin_core::App;

use output::Printer;

#[derive(Parser)]
#[command(
    name = "jianxin",
    version,
    about = "Multi-vendor email client (IMAP/SMTP today, Graph + Gmail API next)",
    long_about = "Read, sync and send mail across Gmail, Outlook, QQ, 163 and any \
                  generic IMAP mailbox.\n\nMailboxes and login state are managed in \
                  config.json, which is written with owner-only permissions."
)]
struct Cli {
    /// Path to config.json (default: $JIANXIN_CONFIG or ~/.config/jianxin/config.json).
    #[arg(long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Machine-readable output.
    #[arg(long, global = true)]
    json: bool,

    /// Which mailbox to use: alias, email, or id prefix.
    #[arg(long, global = true, short = 'a', value_name = "ACCOUNT")]
    account: Option<String>,

    /// Verbose logging (repeat for more).
    #[arg(long, global = true, short = 'v', action = clap::ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    #[command(hide = true)]
    NotificationOpen { url: String },
    /// Run the native tray and latest unread mail panel (requires jianxin-desktop).
    Tray {
        /// Display the panel immediately.
        #[arg(long)]
        show: bool,
        /// Do not start the background mail daemon.
        #[arg(long)]
        no_daemon: bool,
    },
    /// Register the desktop notification identity, inspect it, or send a test.
    Notifications {
        #[arg(long, conflicts_with_all = ["status", "test"])]
        register: bool,
        #[arg(long, conflicts_with = "test")]
        status: bool,
        #[arg(long)]
        test: bool,
    },
    /// Write an example config file.
    Init {
        /// Where to write it (default: the platform config path).
        #[arg(long, value_name = "PATH")]
        path: Option<PathBuf>,
        /// Overwrite an existing file.
        #[arg(long)]
        force: bool,
    },

    /// List configured accounts.
    Accounts,

    /// Check connectivity and credentials for every account (or one with -a).
    Doctor,

    /// Run the read-first MCP tool server on stdio.
    Agent,

    /// Complete OAuth2 device login and store the refresh token.
    Login,

    /// Serve the local web mail reader with synced read flags (loopback only).
    Web {
        #[arg(long, default_value_t = 8787)]
        port: u16,
        /// Initially display this message id or unique prefix.
        #[arg(long)]
        message: Option<String>,
        /// Open the reader in your default browser.
        #[arg(long)]
        open: bool,
        /// Stop after this many idle seconds (0 keeps running).
        #[arg(long, default_value_t = 0)]
        idle_seconds: u64,
    },

    /// Open a message in the browser using a temporary local web reader.
    Open {
        id: String,
        /// Print the URL without launching a browser.
        #[arg(long)]
        no_open: bool,
    },

    /// Refresh mail into the local database.
    Sync {
        /// Ignore stored cursors and re-pull everything.
        #[arg(long)]
        full: bool,
        /// Only this folder (role name, remote name, or id prefix).
        #[arg(long)]
        folder: Option<String>,
    },

    /// Watch accounts by polling and emit persistent new-mail events.
    Watch {
        /// Override config watch.interval_secs.
        #[arg(long, value_name = "SECONDS")]
        interval: Option<u64>,
        /// Override config watch.webhook.
        #[arg(long, value_name = "URL")]
        webhook: Option<String>,
        /// Override config watch.notify.
        #[arg(long)]
        notify: bool,
        /// Emit MCP logging notifications as newline-delimited JSON-RPC.
        #[arg(long)]
        mcp: bool,
        /// Include text and HTML bodies in events.
        #[arg(long)]
        include_body: bool,
    },

    /// Start the background mail poller with desktop notifications enabled.
    Daemon {
        /// Override config watch.interval_secs.
        #[arg(long, value_name = "SECONDS")]
        interval: Option<u64>,
        /// Stop the running background poller.
        #[arg(long, conflicts_with = "status")]
        stop: bool,
        /// Show whether the background poller is running.
        #[arg(long, conflicts_with = "stop")]
        status: bool,
    },

    /// List folders.
    Folders {
        /// Re-list remotely instead of reading the local copy.
        #[arg(long)]
        refresh: bool,
    },

    /// List messages in a folder.
    List {
        /// Folder to list (default: inbox).
        #[arg(default_value = "inbox")]
        folder: String,
        #[arg(long, short = 'n', default_value_t = 20)]
        limit: u32,
        #[arg(long, default_value_t = 0)]
        offset: u32,
        #[arg(long)]
        unread: bool,
    },

    /// Show one message.
    Read {
        /// Message id or unique prefix.
        id: String,
        /// Dump the HTML part instead of the text part.
        #[arg(long)]
        html: bool,
        /// Print the raw headers.
        #[arg(long)]
        headers: bool,
    },

    /// List conversations (threads), most recent activity first.
    Threads {
        #[arg(long, short = 'n', default_value_t = 20)]
        limit: u32,
        #[arg(long, default_value_t = 0)]
        offset: u32,
        /// Only conversations with unread messages.
        #[arg(long)]
        unread: bool,
        /// Drop every stored thread and regroup all messages from their
        /// headers. Use after grouping rules change.
        #[arg(long)]
        rebuild: bool,
    },

    /// Show one conversation, oldest message first.
    Thread {
        /// Thread id/prefix, or any message id/prefix inside it.
        id: String,
        /// Dump the HTML part instead of the text part.
        #[arg(long)]
        html: bool,
    },

    /// Search the local database.
    Search {
        query: String,
        #[arg(long, short = 'n', default_value_t = 20)]
        limit: u32,
        /// Ask the provider instead of searching the local copy.
        #[arg(long)]
        server: bool,
    },

    /// Change flags on a message.
    Flags {
        id: String,
        #[arg(long)]
        read: Option<bool>,
        #[arg(long)]
        star: Option<bool>,
    },

    /// Delete a message.
    Delete { id: String },

    /// Move a message to another folder.
    Move {
        id: String,
        /// Target folder (role name, remote name, or id prefix).
        folder: String,
    },

    /// Send a message.
    Send {
        #[arg(long, required_unless_present = "reply_to")]
        to: Vec<String>,
        #[arg(long)]
        cc: Vec<String>,
        #[arg(long)]
        bcc: Vec<String>,
        #[arg(long, default_value = "")]
        subject: String,
        /// Reply to this message (id or prefix); fills To/Subject/threading
        /// headers when they are not given explicitly.
        #[arg(long, value_name = "ID")]
        reply_to: Option<String>,
        /// Message body. Omit to read from stdin.
        #[arg(long)]
        body: Option<String>,
        /// Read the body from a file.
        #[arg(long, value_name = "PATH")]
        body_file: Option<PathBuf>,
        /// Compose the body in $EDITOR.
        #[arg(long)]
        editor: bool,
        #[arg(long, value_name = "PATH")]
        attach: Vec<PathBuf>,
        /// Save as a draft instead of sending.
        #[arg(long)]
        draft: bool,
    },

    /// Show or drain the outbox.
    Outbox {
        /// Retry delivery of queued/failed jobs.
        #[arg(long)]
        flush: bool,
        /// Move failed jobs back to pending.
        #[arg(long)]
        retry: bool,
    },

    /// Open the terminal UI.
    Tui,
}

fn main() {
    // Load ./.env (current directory only) so JIANXIN_CONFIG and SecretRef
    // env passwords can live there; existing process env always wins.
    let _ = dotenvy::from_filename(".env");

    let cli = Cli::parse();
    init_logging(cli.verbose);

    if let Err(e) = run(cli) {
        eprintln!("error: {e}");
        let mut src = e.source();
        while let Some(s) = src {
            eprintln!("  caused by: {s}");
            src = s.source();
        }
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    let out = Printer::new(cli.json);

    if let Some(Command::Notifications { register, status: _, test }) = &cli.command {
        return notifications::manage(*register, *test);
    }

    if let Some(Command::NotificationOpen { url }) = &cli.command {
        return notifications::open_notification(url);
    }

    // `init` is the one command that runs without a config file.
    if let Some(Command::Init { path, force }) = &cli.command {
        return cmd_init(path.as_deref(), *force, &out);
    }

    let config_path = cli.config.clone().unwrap_or_else(default_config_path);
    if let Some(Command::Tray { show, no_daemon }) = &cli.command {
        let executable = std::env::current_exe()?;
        let name = if cfg!(windows) { "jianxin-desktop.exe" } else { "jianxin-desktop" };
        let sibling = executable.parent().context("cannot locate executable directory")?.join(name);
        let mut command = std::process::Command::new(if sibling.is_file() { sibling } else { name.into() });
        command.arg("--config").arg(std::fs::canonicalize(&config_path)?)
            .arg("--cli").arg(executable);
        if *show { command.arg("--show"); }
        if *no_daemon { command.arg("--no-daemon"); }
        let status = command.status().context("cannot start tray; build jianxin-desktop alongside jianxin")?;
        anyhow::ensure!(status.success(), "desktop tray exited with {status}");
        return Ok(());
    }
    let config = load_config(&config_path)?;
    let app = App::new(config).with_context(|| {
        format!(
            "cannot open the local database (config: {})",
            config_path.display()
        )
    })?;

    let command = cli.command.unwrap_or(Command::Tui);
    let selector = cli.account.as_deref();

    match command {
        Command::NotificationOpen { .. } | Command::Init { .. } | Command::Notifications { .. } | Command::Tray { .. } => unreachable!("handled above"),
        Command::Accounts => cmd_accounts(&app, &out),
        Command::Doctor => cmd_doctor(&app, selector, &out),
        Command::Agent => jianxin_agent::run(app, &config_path),
        Command::Login => cmd_login(&app, &config_path, selector, &out),
        Command::Web { port, message, open, idle_seconds } =>
            web::run(app, &config_path, selector, port, message.as_deref(), open, idle_seconds),
        Command::Open { id, no_open } => web::open_message(&app, &config_path, selector, &id, no_open),
        Command::Sync { full, folder } => cmd_sync(&app, selector, full, folder.as_deref(), &out),
        Command::Watch {
            interval,
            webhook,
            notify,
            mcp,
            include_body,
        } => watch::run(
            &app,
            selector,
            watch::Options {
                interval,
                webhook,
                notify,
                mcp,
                include_body,
                json: out.is_json(),
            },
        ),
        Command::Daemon {
            interval,
            stop,
            status,
        } => {
            let report = daemon::run(daemon::Options {
                config_path: &config_path,
                data_dir: &app.config().data_dir(),
                selector,
                interval,
                stop,
                status,
            })?;
            if out.is_json() {
                out.print_json(&report)
            } else {
                println!("{}", report.human_message());
                Ok(())
            }
        }
        Command::Folders { refresh } => cmd_folders(&app, selector, refresh, &out),
        Command::List {
            folder,
            limit,
            offset,
            unread,
        } => cmd_list(&app, selector, &folder, limit, offset, unread, &out),
        Command::Read { id, html, headers } => cmd_read(&app, selector, &id, html, headers, &out),
        Command::Threads {
            limit,
            offset,
            unread,
            rebuild,
        } => cmd_threads(&app, selector, limit, offset, unread, rebuild, &out),
        Command::Thread { id, html } => cmd_thread(&app, selector, &id, html, &out),
        Command::Search {
            query,
            limit,
            server,
        } => cmd_search(&app, selector, &query, limit, server, &out),
        Command::Flags { id, read, star } => cmd_flags(&app, selector, &id, read, star, &out),
        Command::Delete { id } => cmd_delete(&app, selector, &id, &out),
        Command::Move { id, folder } => cmd_move(&app, selector, &id, &folder, &out),
        Command::Send {
            to,
            cc,
            bcc,
            subject,
            reply_to,
            body,
            body_file,
            editor,
            attach,
            draft,
        } => cmd_send(
            &app,
            selector,
            &to,
            &cc,
            &bcc,
            &subject,
            reply_to.as_deref(),
            body.as_deref(),
            body_file.as_deref(),
            editor,
            &attach,
            draft,
            &out,
        ),
        Command::Outbox { flush, retry } => cmd_outbox(&app, selector, flush, retry, &out),
        Command::Tui => {
            if cli.json {
                bail!("--json is not supported for the interactive TUI");
            }
            jianxin_tui::run_with_config(app, &config_path).context("the terminal UI exited with an error")
        }
    }
}

fn init_logging(verbose: u8) {
    let filter = match verbose {
        0 => "warn",
        1 => "info",
        2 => "jianxin=debug,info",
        _ => "trace",
    };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
        .with_target(false)
        .try_init();
}

fn load_config(path: &Path) -> Result<AppConfig> {
    if !path.exists() {
        let mut help = Cli::command();
        let example = jianxin_config::example_config();
        let pretty = serde_json::to_string_pretty(&example).unwrap_or_default();
        eprintln!("No config file at {}.", path.display());
        eprintln!("Run `jianxin init` to create one. Example:\n\n{pretty}\n");
        help.print_help().ok();
        std::process::exit(2);
    }
    AppConfig::load(path).with_context(|| format!("cannot read {}", path.display()))
}

// --- commands -------------------------------------------------------------

fn cmd_init(path: Option<&Path>, force: bool, out: &Printer) -> Result<()> {
    let path = path
        .map(|p| p.to_path_buf())
        .unwrap_or_else(default_config_path);
    if path.exists() && !force {
        bail!(
            "{} already exists (use --force to overwrite)",
            path.display()
        );
    }
    let config = jianxin_config::example_config();
    config.save(&path)?;
    out.kv("wrote", &path.display().to_string());
    if !out.is_json() {
        eprintln!(
            "\nEdit {} and add your mailboxes.\n\
             Passwords can stay out of the file: use \"password\": {{\"env\": \"MAIL_PASSWORD\"}},\n\
             {{\"command\": \"...\"}} or {{\"keychain\": \"...\"}}.",
            path.display()
        );
    }
    Ok(())
}

fn cmd_accounts(app: &App, out: &Printer) -> Result<()> {
    let accounts = app.accounts()?;
    if out.is_json() {
        out.print_json(&accounts)?;
        return Ok(());
    }
    if accounts.is_empty() {
        eprintln!("No accounts configured. Run `jianxin init`.");
        return Ok(());
    }
    let rows: Vec<Vec<String>> = accounts
        .iter()
        .map(|a| {
            vec![
                a.email.clone(),
                a.provider.to_string(),
                a.auth_type.to_string(),
                short_id(&a.id),
            ]
        })
        .collect();
    out.table(&["EMAIL", "PROVIDER", "AUTH", "ID"], &rows);
    Ok(())
}

fn cmd_doctor(app: &App, selector: Option<&str>, out: &Printer) -> Result<()> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut ok = true;

    for cfg in accounts_to_sync(app, selector)? {
        let account = cfg.to_account();
        match app.provider(&cfg).and_then(|p| p.test_connection()) {
            Ok(detail) => rows.push(vec![
                cfg.alias().to_string(),
                account.provider.to_string(),
                "ok".to_string(),
                detail,
            ]),
            Err(e) => {
                ok = false;
                rows.push(vec![
                    cfg.alias().to_string(),
                    account.provider.to_string(),
                    "FAIL".to_string(),
                    format!("{}: {}", e.code, e.message),
                ]);
            }
        }
    }

    if out.is_json() {
        out.print_json(&rows)?;
    } else {
        out.table(&["ACCOUNT", "PROVIDER", "STATUS", "DETAIL"], &rows);
    }
    if !ok {
        std::process::exit(1);
    }
    Ok(())
}

fn cmd_sync(
    app: &App,
    selector: Option<&str>,
    full: bool,
    only_folder: Option<&str>,
    out: &Printer,
) -> Result<()> {
    for cfg in accounts_to_sync(app, selector)? {
        let account = cfg.to_account();
        let outcome = (|| -> Result<jianxin_core::SyncReport> {
            let provider = app.provider(&cfg)?;
            Ok(match only_folder {
                Some(hint) => {
                    let folder = app.mail().resolve_folder(&cfg, hint)?;
                    let result = app
                        .sync()
                        .sync_folder(&*provider, &account.id, &folder, full)?;
                    let mut r = jianxin_core::SyncReport::new(&account.id);
                    r.folders.push(result);
                    r
                }
                None => app.sync().sync_account(&*provider, &account.id, full)?,
            })
        })();
        let error = match &outcome {
            Err(error) => Some(error.to_string()),
            Ok(report) => report.failures.first().map(|failure| failure.error.to_string()),
        };
        jianxin_core::record_account_sync_status(app.config(), &account.id, error.as_deref());
        let report = outcome?;

        if out.is_json() {
            out.print_json(&serde_json::json!({
                "account": account.email,
                "upserted": report.total_upserted(),
                "removed": report.total_removed(),
                "flagged": report.total_flagged(),
                "failures": report.failures.iter().map(|f| f.error.to_string()).collect::<Vec<_>>(),
            }))?;
        } else {
            for f in &report.folders {
                if f.total_touched() > 0 {
                    println!(
                        "  {}: {} new, {} flagged, {} removed",
                        f.folder_name, f.upserted, f.flagged, f.removed
                    );
                }
            }
            for f in &report.failures {
                eprintln!("  {}: {}", f.folder_name, f.error);
            }
            println!(
                "{}: {} new, {} flagged, {} removed",
                account.email,
                report.total_upserted(),
                report.total_flagged(),
                report.total_removed()
            );
        }
    }
    Ok(())
}

fn cmd_login(app: &App, config_path: &Path, selector: Option<&str>, out: &Printer) -> Result<()> {
    let cfg = app.resolve_account(selector)?;
    let login = jianxin_provider::auth::login_device_code(&cfg, |prompt| {
        eprintln!("Open: {}", prompt.verification_uri);
        eprintln!("Code: {}", prompt.user_code);
        Ok(())
    })?;
    AppConfig::store_refresh_token(config_path, &cfg.email, &login.refresh_token)?;
    out.kv("stored", &config_path.display().to_string());
    Ok(())
}

fn cmd_folders(app: &App, selector: Option<&str>, refresh: bool, out: &Printer) -> Result<()> {
    for cfg in accounts_to_sync(app, selector)? {
        let account = cfg.to_account();
        let folders = if refresh {
            app.sync()
                .refresh_folders(&*app.provider(&cfg)?, &account.id)?;
            app.mail().folders(&account.id)?
        } else {
            app.mail().ensure_folders(&cfg)?
        };

        if out.is_json() {
            out.print_json(&folders)?;
            continue;
        }
        let rows: Vec<Vec<String>> = folders
            .iter()
            .map(|f| {
                vec![
                    f.kind.as_str().to_string(),
                    f.name.clone(),
                    f.unread_count.to_string(),
                    f.total_count.to_string(),
                    short_id(&f.id),
                ]
            })
            .collect();
        println!("{}", account.email);
        out.table(&["ROLE", "NAME", "UNREAD", "TOTAL", "ID"], &rows);
    }
    Ok(())
}

fn cmd_list(
    app: &App,
    selector: Option<&str>,
    folder_hint: &str,
    limit: u32,
    offset: u32,
    unread: bool,
    out: &Printer,
) -> Result<()> {
    let cfg = app.resolve_account(selector)?;
    let folder = app.mail().resolve_folder(&cfg, folder_hint)?;
    let messages = app.mail().list(&folder.id, limit, offset, unread)?;

    if out.is_json() {
        out.print_json(&messages)?;
        return Ok(());
    }
    if messages.is_empty() {
        println!("(no messages in {})", folder.name);
        return Ok(());
    }
    let rows: Vec<Vec<String>> = messages
        .iter()
        .map(|m| {
            vec![
                if m.is_read {
                    " ".to_string()
                } else {
                    "*".to_string()
                },
                short_id(&m.id),
                format_date(m.sort_ts()),
                truncate(&m.sender().display(), 22),
                truncate(&m.subject, 52),
            ]
        })
        .collect();
    out.table(&["", "ID", "DATE", "FROM", "SUBJECT"], &rows);
    Ok(())
}

fn cmd_read(
    app: &App,
    selector: Option<&str>,
    id: &str,
    html: bool,
    headers: bool,
    out: &Printer,
) -> Result<()> {
    let cfg = app.resolve_account(selector)?;
    let account = cfg.to_account();
    let message = app.mail().resolve_message(&account.id, id)?;

    if out.is_json() {
        out.print_json(&message)?;
        return Ok(());
    }

    println!(
        "From:    {}",
        jianxin_domain::join_addresses(&message.from)
    );
    if !message.to.is_empty() {
        println!("To:      {}", jianxin_domain::join_addresses(&message.to));
    }
    if !message.cc.is_empty() {
        println!("Cc:      {}", jianxin_domain::join_addresses(&message.cc));
    }
    println!("Subject: {}", message.subject);
    println!("Date:    {}", format_date(message.sort_ts()));
    if headers {
        if let Some(id) = &message.message_id_header {
            println!("Message-ID: {id}");
        }
        if let Some(r) = &message.in_reply_to {
            println!("In-Reply-To: {r}");
        }
        println!("Remote-Id: {}", message.remote_id);
    }
    if !message.attachments.is_empty() {
        println!(
            "Attach:  {}",
            message
                .attachments
                .iter()
                .map(|a| a.display_name().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    println!();

    let body = if html {
        message.html_body.as_deref().unwrap_or("(no html part)")
    } else {
        message.body_preferred()
    };
    println!("{}", body.trim_end());

    // Reading a message marks it read — that is what every mail client does.
    if !message.is_read {
        let _ = app.mail().mark_read(&cfg, &message.id, true);
    }
    Ok(())
}

fn cmd_threads(
    app: &App,
    selector: Option<&str>,
    limit: u32,
    offset: u32,
    unread: bool,
    rebuild: bool,
    out: &Printer,
) -> Result<()> {
    let cfg = app.resolve_account(selector)?;
    let account = cfg.to_account();

    if rebuild {
        let count = app.mail().rebuild_threads(&account.id)?;
        if out.is_json() {
            out.print_json(&serde_json::json!({ "rebuilt_threads": count }))?;
        } else {
            println!("regrouped into {count} thread(s)");
        }
        return Ok(());
    }

    let threads = app.mail().threads(&account.id, limit, offset, unread)?;

    if out.is_json() {
        out.print_json(&threads)?;
        return Ok(());
    }
    if threads.is_empty() {
        println!("(no threads)");
        return Ok(());
    }
    let rows: Vec<Vec<String>> = threads
        .iter()
        .map(|t| {
            vec![
                if t.unread_count > 0 {
                    "*".to_string()
                } else {
                    " ".to_string()
                },
                short_id(&t.id),
                format_date(t.last_message_at),
                truncate(&t.subject, 48),
                if t.unread_count > 0 {
                    format!("{} ({})", t.message_count, t.unread_count)
                } else {
                    t.message_count.to_string()
                },
            ]
        })
        .collect();
    out.table(&["", "ID", "LAST", "SUBJECT", "MSGS"], &rows);
    Ok(())
}

fn cmd_thread(
    app: &App,
    selector: Option<&str>,
    id: &str,
    html: bool,
    out: &Printer,
) -> Result<()> {
    let cfg = app.resolve_account(selector)?;
    let account = cfg.to_account();
    let thread = app.mail().resolve_thread(&account.id, id)?;
    let detail = app.mail().thread(&account.id, &thread.id)?;

    if out.is_json() {
        out.print_json(&serde_json::json!({
            "thread": detail.thread,
            "messages": detail.messages,
        }))?;
        return Ok(());
    }

    if detail.messages.is_empty() {
        println!("(no messages in thread {})", short_id(&thread.id));
        return Ok(());
    }

    let total = detail.messages.len();
    for (i, message) in detail.messages.iter().enumerate() {
        if i > 0 {
            println!();
        }
        println!("──── #{} of {} ────", i + 1, total);
        println!(
            "From:    {}",
            jianxin_domain::join_addresses(&message.from)
        );
        if !message.to.is_empty() {
            println!("To:      {}", jianxin_domain::join_addresses(&message.to));
        }
        if !message.cc.is_empty() {
            println!("Cc:      {}", jianxin_domain::join_addresses(&message.cc));
        }
        println!("Subject: {}", message.subject);
        println!("Date:    {}", format_date(message.sort_ts()));
        if !message.attachments.is_empty() {
            println!(
                "Attach:  {}",
                message
                    .attachments
                    .iter()
                    .map(|a| a.display_name().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        println!();

        let body = if html {
            message.html_body.as_deref().unwrap_or("(no html part)")
        } else {
            message.body_preferred()
        };
        println!("{}", body.trim_end());
    }

    // Opening a conversation marks it read — same contract as `read`.
    for message in detail.messages.iter().filter(|m| !m.is_read) {
        let _ = app.mail().mark_read(&cfg, &message.id, true);
    }
    Ok(())
}

fn cmd_search(
    app: &App,
    selector: Option<&str>,
    query: &str,
    limit: u32,
    server: bool,
    out: &Printer,
) -> Result<()> {
    let cfg = app.resolve_account(selector)?;
    let account = cfg.to_account();

    let messages = if server {
        let q = jianxin_provider::SearchQuery {
            text: Some(query.to_string()),
            limit,
            ..Default::default()
        };
        app.mail().search_server(&cfg, &q)?
    } else {
        app.mail().search_local(&account.id, query, limit)?
    };

    if out.is_json() {
        out.print_json(&messages)?;
        return Ok(());
    }
    let rows: Vec<Vec<String>> = messages
        .iter()
        .map(|m| {
            vec![
                short_id(&m.id),
                format_date(m.sort_ts()),
                truncate(&m.sender().display(), 22),
                truncate(&m.subject, 52),
            ]
        })
        .collect();
    out.table(&["ID", "DATE", "FROM", "SUBJECT"], &rows);
    Ok(())
}

fn cmd_flags(
    app: &App,
    selector: Option<&str>,
    id: &str,
    read: Option<bool>,
    star: Option<bool>,
    out: &Printer,
) -> Result<()> {
    let cfg = app.resolve_account(selector)?;
    let account = cfg.to_account();
    let message = app.mail().resolve_message(&account.id, id)?;

    if read.is_none() && star.is_none() {
        bail!("pass --read true/false or --star true/false");
    }
    if let Some(read) = read {
        app.mail().mark_read(&cfg, &message.id, read)?;
    }
    if let Some(star) = star {
        app.mail().star(&cfg, &message.id, star)?;
    }
    out.kv("updated", &short_id(&message.id));
    Ok(())
}

fn cmd_delete(app: &App, selector: Option<&str>, id: &str, out: &Printer) -> Result<()> {
    let cfg = app.resolve_account(selector)?;
    let account = cfg.to_account();
    let message = app.mail().resolve_message(&account.id, id)?;
    app.mail().delete(&cfg, &message.id)?;
    out.kv("deleted", &short_id(&message.id));
    Ok(())
}

fn cmd_move(
    app: &App,
    selector: Option<&str>,
    id: &str,
    folder_hint: &str,
    out: &Printer,
) -> Result<()> {
    let cfg = app.resolve_account(selector)?;
    let account = cfg.to_account();
    let message = app.mail().resolve_message(&account.id, id)?;
    let target = app.mail().move_to(&cfg, &message.id, folder_hint)?;
    out.kv(
        "moved",
        &format!("{} -> {}", short_id(&message.id), target.name),
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cmd_send(
    app: &App,
    selector: Option<&str>,
    to: &[String],
    cc: &[String],
    bcc: &[String],
    subject: &str,
    reply_to: Option<&str>,
    body: Option<&str>,
    body_file: Option<&Path>,
    editor: bool,
    attach: &[PathBuf],
    draft: bool,
    out: &Printer,
) -> Result<()> {
    use jianxin_domain::{Address, ComposeAttachment, ComposeMessage};

    let cfg = app.resolve_account(selector)?;
    let account = cfg.to_account();

    let mut compose = ComposeMessage::new();
    compose.from = Some(Address::with_name(
        account.display_name.as_deref().unwrap_or(&account.email),
        &account.email,
    ));
    for a in to {
        compose.to.extend(jianxin_domain::parse_address_list(a));
    }
    for a in cc {
        compose.cc.extend(jianxin_domain::parse_address_list(a));
    }
    for a in bcc {
        compose.bcc.extend(jianxin_domain::parse_address_list(a));
    }
    compose.subject = subject.to_string();

    if let Some(hint) = reply_to {
        let message = app.mail().resolve_message(&account.id, hint)?;
        // Thread the reply: everyone groups on these headers, not just us.
        compose.in_reply_to = message.message_id_header.clone();
        compose.references = jianxin_domain::thread::build_references(&message);
        if compose.subject.is_empty() {
            let s = message.subject.trim();
            compose.subject = if s.to_ascii_lowercase().starts_with("re:") {
                s.to_string()
            } else {
                format!("Re: {s}")
            };
        }
        if compose.to.is_empty() {
            let targets = if !message.reply_to.is_empty() {
                message.reply_to.clone()
            } else {
                message.from.clone()
            };
            compose.to = targets
                .iter()
                .filter(|a| a.email != account.email)
                .cloned()
                .collect();
            if compose.to.is_empty() {
                compose.to = targets;
            }
        }
    }

    let text = if let Some(b) = body {
        b.to_string()
    } else if let Some(path) = body_file {
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?
    } else if editor || atty_stdin() {
        compose_in_editor(app)?
    } else {
        read_stdin()?
    };

    compose.text_body = Some(append_signature(app, text));

    for path in attach {
        let data =
            std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
        let filename = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "attachment".to_string());
        compose.attachments.push(ComposeAttachment::new(
            &filename,
            &ComposeAttachment::guess_mime(&filename),
            data,
        ));
    }

    if draft {
        let message = app.compose().draft(&cfg, &compose)?;
        out.kv("draft", &short_id(&message.id));
        return Ok(());
    }

    let result = app.compose().send(&cfg, &compose)?;
    if out.is_json() {
        out.print_json(&result)?;
    } else {
        println!("sent");
    }
    Ok(())
}

fn cmd_outbox(
    app: &App,
    selector: Option<&str>,
    flush: bool,
    retry: bool,
    out: &Printer,
) -> Result<()> {
    let cfg = app.resolve_account(selector)?;

    if retry {
        let n = app.compose().retry_failed(&cfg)?;
        out.kv("requeued", &n.to_string());
        return Ok(());
    }

    if flush {
        let report = app.compose().flush(&cfg)?;
        if out.is_json() {
            out.print_json(&serde_json::json!({
                "sent": report.sent,
                "failed": report.failed,
                "errors": report.errors.iter().map(|(_, e)| e.to_string()).collect::<Vec<_>>(),
            }))?;
        } else {
            println!("sent {}, failed {}", report.sent, report.failed);
            for (_, e) in &report.errors {
                eprintln!("  {e}");
            }
        }
        return Ok(());
    }

    let jobs = app.compose().pending(&cfg)?;
    if out.is_json() {
        out.print_json(&jobs)?;
        return Ok(());
    }
    if jobs.is_empty() {
        println!("(outbox is empty)");
        return Ok(());
    }
    let rows: Vec<Vec<String>> = jobs
        .iter()
        .map(|j| {
            vec![
                short_id(&j.id),
                j.status.to_string(),
                j.retry_count.to_string(),
                j.last_error.clone().unwrap_or_default(),
            ]
        })
        .collect();
    out.table(&["ID", "STATUS", "RETRIES", "LAST ERROR"], &rows);
    Ok(())
}

// --- helpers --------------------------------------------------------------

/// Every configured account, or just the selected one.
fn accounts_to_sync(
    app: &App,
    selector: Option<&str>,
) -> Result<Vec<jianxin_config::AccountConfig>> {
    if app.account_configs().is_empty() {
        bail!("no accounts configured — run `jianxin init`");
    }
    match selector {
        Some(_) => Ok(vec![app.resolve_account(selector)?]),
        None => Ok(app.account_configs().to_vec()),
    }
}

fn append_signature(app: &App, body: String) -> String {
    match app.config().general.compose.signature.as_deref() {
        Some(sig) if !sig.trim().is_empty() => format!("{body}\n-- \n{sig}\n"),
        _ => body,
    }
}

/// Open `$EDITOR` on a temporary file and read the result back.
fn compose_in_editor(app: &App) -> Result<String> {
    let editor = app.config().editor();
    let path = std::env::temp_dir().join(format!("jianxin-{}.txt", jianxin_domain::new_id()));
    std::fs::write(&path, "\n").context("cannot create a temporary draft file")?;

    let status = std::process::Command::new(&editor)
        .arg(&path)
        .status()
        .with_context(|| format!("cannot launch editor `{editor}`"))?;
    if !status.success() {
        bail!("`{editor}` exited with {status}");
    }

    let body = std::fs::read_to_string(&path).context("cannot read the draft back")?;
    let _ = std::fs::remove_file(&path);
    if body.trim().is_empty() {
        bail!("empty message, not sending");
    }
    Ok(body)
}

fn read_stdin() -> Result<String> {
    use std::io::Read;
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .context("cannot read stdin")?;
    Ok(buf)
}

fn atty_stdin() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
}

fn short_id(id: &str) -> String {
    if id.len() > 8 {
        id[..8].to_string()
    } else {
        id.to_string()
    }
}

fn truncate(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    let cut: String = chars[..max.saturating_sub(1)].iter().collect();
    format!("{cut}…")
}

fn format_date(ts: i64) -> String {
    if ts == 0 {
        return "        ".to_string();
    }
    use chrono::{Local, TimeZone};
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|d| d.format("%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "        ".to_string())
}

#[cfg(test)]
mod cli_tests {
    use super::*;

    #[test]
    fn notification_modes_are_exclusive() {
        assert!(Cli::try_parse_from(["jianxin", "notifications", "--register", "--test"]).is_err());
        assert!(Cli::try_parse_from(["jianxin", "notifications", "--status", "--test"]).is_err());
        assert!(matches!(Cli::try_parse_from(["jianxin", "notifications", "--test"]).unwrap().command,
            Some(Command::Notifications { test: true, .. })));
    }

    #[test]
    fn web_defaults_do_not_launch_browser() {
        let cli = Cli::try_parse_from(["jianxin", "web"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Web { port: 8787, open: false, idle_seconds: 0, .. })));
        let cli = Cli::try_parse_from(["jianxin", "open", "mail-id", "--no-open"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Open { no_open: true, .. })));
    }

    #[test]
    fn daemon_is_a_direct_command() {
        let cli = Cli::try_parse_from(["jianxin", "daemon", "--interval", "60"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Daemon {
                interval: Some(60),
                stop: false,
                status: false,
            })
        ));
    }
}
