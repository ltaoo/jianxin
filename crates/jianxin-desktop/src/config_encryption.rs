use crate::{config_crypto, theme::Tokens};
use anyhow::Result;
use gpui_kit::{
    component::{
        button::{Button, ButtonVariants},
        input::{Input, InputContentType, InputEvent, InputState, Textarea, TextareaState},
        Disableable,
    },
    prelude::FluentBuilder,
    *,
};
use std::path::PathBuf;
use zeroize::Zeroizing;

/// Shared by both tray entries so opening the tool twice preserves the current form.
pub struct EncryptionWindow {
    config_path: PathBuf,
    window: Option<AnyWindowHandle>,
}

impl EncryptionWindow {
    pub fn new(config_path: PathBuf) -> Self {
        Self {
            config_path,
            window: None,
        }
    }

    pub fn show(&mut self, cx: &mut App) -> Result<()> {
        if let Some(window) = self.window {
            if window
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
            {
                cx.activate(true);
                return Ok(());
            }
            self.window = None;
        }
        let t = Tokens::load();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(
                    t.length("layout", "configEncryptionWidth"),
                    t.length("layout", "configEncryptionHeight"),
                ),
                cx,
            ))),
            titlebar: Some(TitlebarOptions {
                title: Some("简信 · 配置加密".into()),
                ..Default::default()
            }),
            window_min_size: Some(size(
                t.length("layout", "notification"),
                t.length("layout", "configEncryptionHeight"),
            )),
            app_id: Some("com.jianxin.mail".into()),
            ..Default::default()
        };
        let path = self.config_path.clone();
        let (window, _) = gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| ConfigEncryption::new(path, window, cx))
        })?;
        self.window = Some(window);
        cx.activate(true);
        Ok(())
    }
}

struct ConfigEncryption {
    config_path: PathBuf,
    source: Entity<InputState>,
    password: Entity<InputState>,
    output: Entity<TextareaState>,
    busy: bool,
    has_output: bool,
    copied: bool,
    error: Option<String>,
    tokens: Tokens,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl ConfigEncryption {
    fn new(config_path: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let source = cx
            .new(|cx| InputState::new(window, cx).default_value(config_path.display().to_string()));
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("输入用于手机解密的密钥")
        });
        let output = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(8)
                .placeholder("输入密钥后，点击「生成密文」")
        });
        let focus = cx.focus_handle();
        password.update(cx, |input, cx| input.focus(window, cx));
        let change = cx.subscribe_in(
            &password,
            window,
            |page, _, event, window, cx| match event {
                InputEvent::Change if !page.busy => {
                    page.clear_output(window, cx);
                    page.error = None;
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => page.generate(window, cx),
                _ => {}
            },
        );
        let mut tokens = Tokens::load();
        tokens.apply(window.appearance(), cx);
        let appearance = cx.observe_window_appearance(window, |page, window, cx| {
            page.tokens.apply(window.appearance(), cx);
            cx.notify();
        });
        Self {
            config_path,
            source,
            password,
            output,
            busy: false,
            has_output: false,
            copied: false,
            error: None,
            tokens,
            focus,
            _subscriptions: vec![change, appearance],
        }
    }

    fn clear_output(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.has_output = false;
        self.copied = false;
        self.output
            .update(cx, |input, cx| input.set_value("", window, cx));
    }

    fn generate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.clear_output(window, cx);
        let password = Zeroizing::new(self.password.read(cx).text().to_string());
        if let Err(error) = config_crypto::validate_password(&password) {
            self.error = Some(error.to_string());
            self.password
                .update(cx, |input, cx| input.focus(window, cx));
            cx.notify();
            return;
        }
        self.busy = true;
        self.error = None;
        let path = self.config_path.clone();
        let encryption = cx
            .background_executor()
            .spawn(async move { config_crypto::encrypt_current_config(&path, &password) });
        cx.spawn_in(window, async move |page, cx| {
            let result = encryption.await;
            let _ = page.update_in(cx, |page, window, cx| {
                page.busy = false;
                match result {
                    Ok(ciphertext) => {
                        page.output
                            .update(cx, |input, cx| input.set_value(ciphertext, window, cx));
                        page.has_output = true;
                    }
                    Err(error) => page.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        if self.has_output {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.output.read(cx).value().to_string(),
            ));
            self.copied = true;
            cx.notify();
        }
    }
}

impl Render for ConfigEncryption {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.tokens;
        let source = div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap(t.length("spacing", "2"))
            .child("当前配置文件")
            .child(
                Input::new(&self.source)
                    .readonly(true)
                    .aria_label("当前使用的配置文件路径")
                    .h(t.length("layout", "target")),
            )
            .child(
                div()
                    .text_size(t.length("type", "caption"))
                    .text_color(t.color("muted"))
                    .child("每次生成时重新读取此文件。"),
            );
        let password = div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap(t.length("spacing", "2"))
            .child("加密密钥")
            .child(
                Input::new(&self.password)
                    .content_type(InputContentType::NewPassword)
                    .aria_label("加密密钥")
                    .disabled(self.busy)
                    .h(t.length("layout", "target")),
            )
            .child(
                div()
                    .text_size(t.length("type", "caption"))
                    .text_color(t.color("muted"))
                    .child("手机解密时输入相同密钥。建议使用长且独立的密钥。"),
            );
        let generate = Button::new("generate-config-ciphertext")
            .primary()
            .label(if self.busy {
                "正在生成…"
            } else {
                "生成密文"
            })
            .loading(self.busy)
            .h(t.length("layout", "target"))
            .on_click(cx.listener(|page, _, window, cx| page.generate(window, cx)));
        let copy = Button::new("copy-config-ciphertext")
            .ghost()
            .label(if self.copied {
                "已复制"
            } else {
                "复制密文"
            })
            .disabled(!self.has_output || self.busy)
            .h(t.length("layout", "target"))
            .on_click(cx.listener(|page, _, _, cx| page.copy(cx)));
        let output = div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap(t.length("spacing", "2"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child("密文")
                    .child(copy),
            )
            .child(
                Textarea::new(&self.output)
                    .readonly(true)
                    .aria_label("生成的 Base64 密文")
                    .h(t.length("layout", "configEncryptionOutput")),
            );
        div()
            .size_full().bg(t.color("canvas")).text_color(t.color("text"))
            .text_size(t.length("type", "control")).track_focus(&self.focus)
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, window, _| {
                if event.keystroke.key == "escape" {
                    window.remove_window();
                }
            }))
            .child(
                div().id("config-encryption-page").size_full().overflow_y_scroll()
                    .flex().flex_col().gap(t.length("spacing", "4")).p(t.length("spacing", "6"))
                    .child(
                        div().flex_shrink_0().text_size(t.length("type", "heading"))
                            .font_weight(FontWeight::SEMIBOLD).child("配置加密"),
                    )
                    .child(
                        div().flex_shrink_0().text_color(t.color("muted"))
                            .child("使用当前邮箱配置生成密文，供手机从 URL 导入。"),
                    )
                    .child(source).child(password)
                    .child(
                        div().flex().flex_shrink_0().items_center().gap(t.length("spacing", "3"))
                            .child(generate)
                            .child(div().text_color(t.color("muted")).child("AES-256-GCM")),
                    )
                    .when_some(self.error.clone(), |content, error| {
                        content.child(div().flex_shrink_0().text_color(t.color("danger")).child(error))
                    })
                    .child(output)
                    .child(
                        div().flex_shrink_0().text_size(t.length("type", "caption"))
                            .text_color(t.color("muted"))
                            .child("将密文保存为文本文件并上传到 CDN；手机选择 AES-256-GCM，再填写 HTTPS 地址和密钥。账号凭据需为字符串，手机不读取电脑上的凭据引用。"),
                    ),
            )
    }
}
