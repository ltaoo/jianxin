use crate::model::Snapshot;
use anyhow::Result;

#[cfg(any(target_os = "macos", target_os = "windows"))]
const QUIT_MENU_ID: &str = "jianxin.quit";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Open(Option<String>),
    Mailbox(String),
    Refresh,
    Quit,
}

#[derive(Clone)]
enum Entry {
    Label(String),
    Item(String, Action),
    Separator,
    Mailboxes(Vec<crate::model::MailboxRow>),
}

fn entries(snapshot: &Snapshot, status: Option<&str>) -> Vec<Entry> {
    let mut entries = vec![Entry::Label(format!(
        "最近 10 封未读邮件 · 共 {} 封",
        snapshot.total
    ))];
    if let Some(error) = status.or(snapshot.error.as_deref()) {
        entries.push(Entry::Label(error.into()));
    }
    if snapshot.rows.is_empty() {
        entries.push(Entry::Label(
            if snapshot.loaded {
                "暂无未读邮件"
            } else {
                "正在加载未读邮件…"
            }
            .into(),
        ));
    } else {
        entries.extend(
            snapshot.rows.iter().take(10).map(|row| {
                Entry::Item(menu_title(&row.subject), Action::Open(Some(row.id.clone())))
            }),
        );
    }
    entries.extend([
        Entry::Separator,
        Entry::Mailboxes(snapshot.mailboxes.clone()),
        Entry::Item("打开邮件阅读器".into(), Action::Open(None)),
        Entry::Item("立即收信 / 刷新菜单".into(), Action::Refresh),
        Entry::Item("退出应用".into(), Action::Quit),
    ]);
    entries
}

fn mailbox_title(mailbox: &crate::model::MailboxRow) -> String {
    let time = mailbox
        .last_sync_at
        .and_then(|time| chrono::DateTime::from_timestamp(time, 0))
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        });
    format!(
        "{} · {} · {}",
        mailbox
            .email
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
        time.map(|time| format!("最近同步 {time}"))
            .unwrap_or_else(|| "尚无同步记录".into()),
        mailbox.status
    )
}

fn menu_title(subject: &str) -> String {
    let clean = subject.split_whitespace().collect::<Vec<_>>().join(" ");
    let clean: String = clean
        .chars()
        .filter(|character| !character.is_control())
        .collect();
    if clean.is_empty() {
        return "（无主题）".into();
    }
    let mut chars = clean.chars();
    let mut title: String = chars.by_ref().take(80).collect();
    if chars.next().is_some() {
        title.push('…');
    }
    title
}

pub struct Tray {
    native: NativeTray,
    snapshot: Snapshot,
    status: Option<String>,
}

impl Tray {
    pub fn new() -> Result<Self> {
        let snapshot = Snapshot::default();
        Ok(Self {
            native: NativeTray::new(entries(&snapshot, None))?,
            snapshot,
            status: None,
        })
    }
    pub fn update(&mut self, snapshot: Snapshot, status: Option<String>) -> Result<()> {
        if self.snapshot != snapshot || self.status != status {
            self.native
                .update(entries(&snapshot, status.as_deref()), snapshot.total)?;
            self.snapshot = snapshot;
            self.status = status;
        }
        Ok(())
    }
    pub fn actions(&mut self) -> Vec<Action> {
        self.native.actions()
    }
    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.clone()
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub fn record_status(&self, path: &std::path::Path) {
        #[cfg(target_os = "macos")]
        crate::macos::record_status(&self.native.icon, path);
        #[cfg(windows)]
        {
            let status = serde_json::json!({"pid":std::process::id(), "tray_created":true, "web_embedded":true});
            if let Ok(bytes) = serde_json::to_vec(&status) {
                let _ = std::fs::write(path, bytes);
            }
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
struct NativeTray {
    icon: tray_icon::TrayIcon,
    quit_item: tray_icon::menu::MenuItem,
    actions: std::collections::HashMap<tray_icon::menu::MenuId, Action>,
    incoming: std::sync::mpsc::Receiver<tray_icon::menu::MenuEvent>,
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
impl NativeTray {
    fn menu(
        entries: Vec<Entry>,
        quit_item: &tray_icon::menu::MenuItem,
    ) -> Result<(
        tray_icon::menu::Menu,
        std::collections::HashMap<tray_icon::menu::MenuId, Action>,
    )> {
        use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
        let menu = Menu::new();
        let mut actions = std::collections::HashMap::new();
        for entry in entries {
            match entry {
                Entry::Mailboxes(mailboxes) => {
                    let submenu = Submenu::new("邮箱", true);
                    if mailboxes.is_empty() {
                        submenu.append(&MenuItem::new("尚未添加邮箱", false, None))?;
                    }
                    for mailbox in mailboxes {
                        let item =
                            MenuItem::new(mailbox_title(&mailbox).replace('&', "&&"), true, None);
                        actions.insert(item.id().clone(), Action::Mailbox(mailbox.id));
                        submenu.append(&item)?;
                    }
                    menu.append(&submenu)?;
                }
                Entry::Separator => menu.append(&PredefinedMenuItem::separator())?,
                Entry::Label(text) => {
                    menu.append(&MenuItem::new(text.replace('&', "&&"), false, None))?
                }
                Entry::Item(text, action) => {
                    let item = if action == Action::Quit {
                        // Keep the native command ID too: Windows WM_COMMAND
                        // carries only 16 bits, while muda's counter keeps growing.
                        quit_item.set_text(text.replace('&', "&&"));
                        quit_item.clone()
                    } else {
                        MenuItem::new(text.replace('&', "&&"), true, None)
                    };
                    actions.insert(item.id().clone(), action);
                    menu.append(&item)?;
                }
            }
        }
        Ok((menu, actions))
    }
    fn new(entries: Vec<Entry>) -> Result<Self> {
        let (sender, incoming) = std::sync::mpsc::channel();
        tray_icon::menu::MenuEvent::set_event_handler(Some(
            move |event: tray_icon::menu::MenuEvent| {
                // Runs at native menu activation, even if the app's polling loop
                // subsequently blocks. Quit has a stable ID across menu refreshes.
                if event.id.as_ref() == QUIT_MENU_ID {
                    crate::shutdown::request_exit();
                }
                let _ = sender.send(event);
            },
        ));
        let quit_item = tray_icon::menu::MenuItem::with_id(QUIT_MENU_ID, "", true, None);
        let (menu, actions) = Self::menu(entries, &quit_item)?;
        #[cfg(target_os = "macos")]
        let icon_bytes = include_bytes!("../../../assets/jianxin-tray-template.png").as_slice();
        #[cfg(target_os = "windows")]
        let icon_bytes = include_bytes!("../../../assets/jianxin-tray.png").as_slice();
        let image =
            image::load_from_memory_with_format(icon_bytes, image::ImageFormat::Png)?.into_rgba8();
        let (width, height) = image.dimensions();
        let icon = tray_icon::Icon::from_rgba(image.into_raw(), width, height)?;
        let builder = tray_icon::TrayIconBuilder::new()
            .with_icon(icon)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(true)
            .with_tooltip("简信 · 正在加载未读邮件");
        #[cfg(target_os = "macos")]
        let builder = builder.with_autosave_name("com.jianxin.mail.tray");
        let icon = builder.build()?;
        #[cfg(target_os = "macos")]
        crate::macos::configure(&icon)?;
        Ok(Self {
            icon,
            quit_item,
            actions,
            incoming,
        })
    }
    fn update(&mut self, entries: Vec<Entry>, total: u64) -> Result<()> {
        let (menu, actions) = Self::menu(entries, &self.quit_item)?;
        self.icon.set_menu(Some(Box::new(menu)));
        self.actions = actions;
        self.icon
            .set_tooltip(Some(format!("简信 · {total} 封未读邮件")))?;
        #[cfg(target_os = "macos")]
        self.icon.set_title(if total > 0 {
            Some(total.to_string())
        } else {
            None
        });
        Ok(())
    }
    fn actions(&mut self) -> Vec<Action> {
        // Drain native events before replacing a menu, preserving the selected mail.
        while tray_icon::TrayIconEvent::receiver().try_recv().is_ok() {}
        self.incoming
            .try_iter()
            .filter_map(|event| self.actions.get(&event.id).cloned())
            .collect()
    }
}

#[cfg(target_os = "linux")]
struct LinuxTray {
    entries: Vec<Entry>,
    total: u64,
    icon: ksni::Icon,
    sender: std::sync::mpsc::Sender<Action>,
}

#[cfg(target_os = "linux")]
impl ksni::Tray for LinuxTray {
    // Advertise IsMenu so left-click opens the menu in StatusNotifier hosts.
    const MENU_ON_ACTIVATE: bool = true;
    fn id(&self) -> String {
        "com.jianxin.mail".into()
    }
    fn title(&self) -> String {
        format!("简信 · {} 封未读邮件", self.total)
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![self.icon.clone()]
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: self.title(),
            ..Default::default()
        }
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        self.entries
            .iter()
            .map(|entry| {
                let (label, action) = match entry {
                    Entry::Mailboxes(mailboxes) => {
                        return ksni::menu::SubMenu {
                            label: "邮箱".into(),
                            submenu: if mailboxes.is_empty() {
                                vec![ksni::menu::StandardItem {
                                    label: "尚未添加邮箱".into(),
                                    enabled: false,
                                    ..Default::default()
                                }
                                .into()]
                            } else {
                                mailboxes
                                    .iter()
                                    .map(|mailbox| {
                                        let id = mailbox.id.clone();
                                        ksni::menu::StandardItem {
                                            label: mailbox_title(mailbox).replace('_', "__"),
                                            activate: Box::new(move |tray: &mut Self| {
                                                let _ =
                                                    tray.sender.send(Action::Mailbox(id.clone()));
                                            }),
                                            ..Default::default()
                                        }
                                        .into()
                                    })
                                    .collect()
                            },
                            ..Default::default()
                        }
                        .into()
                    }
                    Entry::Separator => return ksni::MenuItem::Separator,
                    Entry::Label(label) => (label, None),
                    Entry::Item(label, action) => (label, Some(action.clone())),
                };
                ksni::menu::StandardItem {
                    label: label.replace('_', "__"),
                    enabled: action.is_some(),
                    activate: Box::new(move |tray: &mut Self| {
                        if let Some(action) = &action {
                            if *action == Action::Quit {
                                crate::shutdown::request_exit();
                            }
                            let _ = tray.sender.send(action.clone());
                        }
                    }),
                    ..Default::default()
                }
                .into()
            })
            .collect()
    }
}

#[cfg(target_os = "linux")]
struct NativeTray {
    handle: ksni::blocking::Handle<LinuxTray>,
    incoming: std::sync::mpsc::Receiver<Action>,
}

#[cfg(target_os = "linux")]
impl NativeTray {
    fn new(entries: Vec<Entry>) -> Result<Self> {
        use ksni::blocking::TrayMethods;
        let (sender, incoming) = std::sync::mpsc::channel();
        let image = image::load_from_memory_with_format(
            include_bytes!("../../../assets/jianxin-tray.png"),
            image::ImageFormat::Png,
        )?
        .into_rgba8();
        let (width, height) = image.dimensions();
        let mut data = image.into_raw();
        for pixel in data.chunks_exact_mut(4) {
            pixel.rotate_right(1);
        }
        let handle = LinuxTray {
            entries,
            total: 0,
            icon: ksni::Icon {
                width: width as i32,
                height: height as i32,
                data,
            },
            sender,
        }
        .assume_sni_available(true)
        .spawn()?;
        Ok(Self { handle, incoming })
    }
    fn update(&mut self, entries: Vec<Entry>, total: u64) -> Result<()> {
        self.handle.update(move |tray| {
            tray.entries = entries;
            tray.total = total;
        });
        Ok(())
    }
    fn actions(&mut self) -> Vec<Action> {
        self.incoming.try_iter().collect()
    }
}

#[cfg(target_os = "linux")]
impl Drop for NativeTray {
    fn drop(&mut self) {
        self.handle.shutdown().wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn quit_menu_survives_native_command_id_exhaustion() {
        use tray_icon::menu::{ContextMenu, Menu, MenuEvent, MenuItem};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, GetMenuItemCount, GetMenuItemID, SendMessageW,
            WM_COMMAND,
        };

        fn activate_quit(menu: &Menu) {
            // Exercise the same subclass and 16-bit command used by a tray click.
            let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
            unsafe {
                let window = CreateWindowExW(
                    0,
                    class.as_ptr(),
                    std::ptr::null(),
                    0,
                    0,
                    0,
                    0,
                    0,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                );
                assert!(!window.is_null());
                let popup = menu.hpopupmenu() as _;
                let command = GetMenuItemID(popup, GetMenuItemCount(popup) - 1);
                assert_ne!(command, u32::MAX);
                menu.attach_menu_subclass_for_hwnd(window as _);
                SendMessageW(window, WM_COMMAND, (command & 0xffff) as usize, 0);
                menu.detach_menu_subclass_from_hwnd(window as _);
                DestroyWindow(window);
            }
            assert_eq!(
                MenuEvent::receiver()
                    .try_recv()
                    .expect("native quit click must emit an event")
                    .id
                    .as_ref(),
                QUIT_MENU_ID,
            );
        }

        let quit_item = MenuItem::with_id(QUIT_MENU_ID, "", true, None);
        let (menu, _) = NativeTray::menu(entries(&Snapshot::default(), None), &quit_item).unwrap();
        activate_quit(&menu);
        drop(menu);
        // Menu rebuilds consume numeric IDs even when the string ID is fixed.
        for _ in 0..=u16::MAX {
            drop(MenuItem::new("refresh", true, None));
        }
        let (menu, actions) =
            NativeTray::menu(entries(&Snapshot::default(), None), &quit_item).unwrap();
        assert_eq!(actions.get(&QUIT_MENU_ID.into()), Some(&Action::Quit));
        activate_quit(&menu);
    }

    #[test]
    fn mailbox_submenu_includes_unsynced_accounts_and_latest_attempt_times() {
        let mailboxes = vec![
            crate::model::MailboxRow {
                id: "first".into(),
                email: "a@example.com".into(),
                last_sync_at: None,
                status: "未同步".into(),
            },
            crate::model::MailboxRow {
                id: "second".into(),
                email: "b@example.com".into(),
                last_sync_at: Some(1700000000),
                status: "登录失败".into(),
            },
        ];
        let menu = entries(
            &Snapshot {
                mailboxes: mailboxes.clone(),
                ..Default::default()
            },
            None,
        );
        let Entry::Mailboxes(rows) = menu
            .iter()
            .find(|entry| matches!(entry, Entry::Mailboxes(_)))
            .unwrap()
        else {
            unreachable!()
        };
        assert_eq!(rows, &mailboxes);
        assert!(mailbox_title(&rows[0]).contains("尚无同步记录"));
        let title = mailbox_title(&rows[1]);
        assert!(title.contains("最近同步 2023-"));
        assert!(title.contains("登录失败"));
    }

    #[test]
    fn menu_keeps_title_only_and_routes_duplicate_titles_by_id() {
        let snapshot = Snapshot {
            loaded: true,
            total: 12,
            rows: (0..12)
                .map(|index| crate::model::MailRow {
                    id: format!("mail-{index}"),
                    subject: "Same & subject".into(),
                })
                .collect(),
            error: None,
            mailboxes: vec![],
        };
        let menu = entries(&snapshot, None);
        let mails: Vec<_> = menu
            .iter()
            .filter_map(|entry| match entry {
                Entry::Item(title, Action::Open(Some(id))) => Some((title.as_str(), id.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(mails.len(), 10);
        assert_eq!(mails[0], ("Same & subject", "mail-0"));
        assert_eq!(mails[9].1, "mail-9");
        assert_eq!(menu_title(" \n\t "), "（无主题）");
        assert_eq!(menu_title("First\nsecond\tthird"), "First second third");
        assert_eq!(menu_title(&"标题".repeat(100)).chars().count(), 81);
    }
}
