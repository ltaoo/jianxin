use crate::{model::Snapshot, theme::Tokens, Request};
use gpui_kit::{
    component::{
        button::{Button, ButtonVariants},
        menu::{ContextMenuExt, PopupMenuItem},
    },
    *,
};
use std::{cell::RefCell, rc::Rc, sync::mpsc::SyncSender, time::Instant};

pub type ClosedAt = Rc<RefCell<Option<Instant>>>;

pub struct Panel {
    pub snapshot: Snapshot,
    pub action_error: Option<String>,
    requests: SyncSender<Request>,
    tokens: Tokens,
    focus: FocusHandle,
    _activation: Subscription,
    _appearance: Subscription,
    closed_at: ClosedAt,
}

impl Panel {
    pub fn new(
        snapshot: Snapshot,
        requests: SyncSender<Request>,
        closed_at: ClosedAt,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let activation = cx.observe_window_activation(window, |panel, window, _| {
            if !window.is_window_active() {
                panel.dismiss(window);
            }
        });
        let mut tokens = Tokens::load();
        tokens.apply(window.appearance(), cx);
        let appearance = cx.observe_window_appearance(window, |panel, window, cx| {
            panel.tokens.apply(window.appearance(), cx);
            cx.notify();
        });
        Self {
            snapshot,
            action_error: None,
            requests,
            tokens,
            focus,
            _activation: activation,
            _appearance: appearance,
            closed_at,
        }
    }
    fn dismiss(&mut self, window: &mut Window) {
        *self.closed_at.borrow_mut() = Some(Instant::now());
        window.remove_window();
    }
    fn request(&mut self, request: Request, cx: &mut Context<Self>) {
        if self.requests.try_send(request).is_err() {
            self.action_error = Some("操作正在进行，请稍后重试".into());
        } else {
            self.action_error = None;
        }
        cx.notify();
    }
}

impl Render for Panel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.tokens;
        let mut list = div()
            .id("unread-list")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        for row in &self.snapshot.rows {
            let menu_id = row.id.clone();
            let panel = cx.weak_entity();
            list = list.child(
                div()
                    .id(SharedString::from(format!("mail-menu-{}", row.id)))
                    .child(
                        div()
                            .id(SharedString::from(format!("mail-{}", row.id)))
                            .w_full()
                            .h_auto()
                            .min_h(t.length("layout", "target"))
                            .p(t.length("spacing", "3"))
                            .rounded_none()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(t.length("spacing", "2"))
                                    .w_full()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .text_size(t.length("type", "control"))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(t.color("text"))
                                            .truncate()
                                            .child(row.subject.clone()),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .justify_between()
                                            .gap(t.length("spacing", "2"))
                                            .text_size(t.length("type", "caption"))
                                            .text_color(t.color("muted"))
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .truncate()
                                                    .child(row.account.clone()),
                                            )
                                            .child(div().flex_shrink_0().child(row.time.clone())),
                                    ),
                            ),
                    )
                    .context_menu(move |menu, _, _| {
                        let panel = panel.clone();
                        let id = menu_id.clone();
                        menu.item(
                            PopupMenuItem::new("在浏览器中打开").on_click(move |_, _, cx| {
                                let _ = panel.update(cx, |panel, cx| {
                                    panel.request(Request::Open(Some(id.clone())), cx);
                                });
                            }),
                        )
                    }),
            );
        }
        if !self.snapshot.loaded {
            list = list.child(div().p(t.length("spacing", "4")).child("正在加载未读邮件…"));
        } else if let Some(error) = &self.snapshot.error {
            list = list.child(
                div()
                    .p(t.length("spacing", "4"))
                    .text_color(t.color("danger"))
                    .child(error.clone()),
            );
        } else if self.snapshot.rows.is_empty() {
            list = list.child(
                div()
                    .p(t.length("spacing", "4"))
                    .text_color(t.color("muted"))
                    .child("暂无未读邮件。后台收信后会自动更新。"),
            );
        }
        let mut content = div()
            .size_full()
            .flex()
            .flex_col()
            .bg(t.color("canvas"))
            .text_color(t.color("text"))
            .text_size(t.length("type", "control"))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|panel, event: &KeyDownEvent, window, _| {
                if event.keystroke.key == "escape" {
                    panel.dismiss(window);
                }
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .p(t.length("spacing", "3"))
                    .border_b_1()
                    .border_color(t.color("line"))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("未读邮件 · {}", self.snapshot.total)),
                    )
                    .child(
                        Button::new("close")
                            .ghost()
                            .label("关闭")
                            .on_click(cx.listener(|panel, _, window, _| panel.dismiss(window))),
                    ),
            )
            .child(list);
        if let Some(error) = &self.action_error {
            content = content.child(
                div()
                    .p(t.length("spacing", "3"))
                    .text_color(t.color("danger"))
                    .child(error.clone()),
            );
        }
        content.child(
            div()
                .flex()
                .flex_col()
                .gap(t.length("spacing", "2"))
                .p(t.length("spacing", "3"))
                .border_t_1()
                .border_color(t.color("line"))
                .child(
                    div()
                        .text_size(t.length("type", "caption"))
                        .text_color(t.color("muted"))
                        .child(format!(
                            "显示最新 {} 封 · 每 5 秒更新",
                            crate::model::LIST_LIMIT
                        )),
                )
                .child(
                    div()
                        .flex()
                        .gap(t.length("spacing", "2"))
                        .child(
                            Button::new("reader")
                                .primary()
                                .label("打开邮件阅读器")
                                .h(t.length("layout", "target"))
                                .on_click(cx.listener(|panel, _, _, cx| {
                                    panel.request(Request::Open(None), cx)
                                })),
                        )
                        .child(
                            Button::new("refresh")
                                .ghost()
                                .label("刷新")
                                .h(t.length("layout", "target"))
                                .on_click(cx.listener(|panel, _, _, cx| {
                                    panel.request(Request::Refresh, cx)
                                })),
                        )
                        .child(div().flex_1())
                        .child(
                            Button::new("quit")
                                .ghost()
                                .label("退出")
                                .accessibility_label("退出简信桌面应用")
                                .tooltip("退出应用并停止后台收信")
                                .h(t.length("layout", "target"))
                                .on_click(
                                    cx.listener(|panel, _, _, cx| panel.request(Request::Quit, cx)),
                                ),
                        ),
                ),
        )
    }
}
