use crate::instance::OpenRequest;
use anyhow::{bail, Context, Result};
use std::{ffi::OsString, path::PathBuf};

#[derive(Default)]
pub struct Options {
    pub config: Option<PathBuf>,
    pub open: Option<OpenRequest>,
    pub no_daemon: bool,
}
impl Options {
    pub fn parse() -> Result<Self> {
        Self::from_args(std::env::args_os().skip(1))
    }
    fn from_args(args: impl IntoIterator<Item = OsString>) -> Result<Self> {
        let mut options = Self::default();
        let mut args = args.into_iter();
        while let Some(argument) = args.next() {
            let value = argument.to_str().context("启动参数无效")?;
            match value {
                "--config" => {
                    options.config = Some(args.next().context("--config 缺少路径")?.into())
                }
                "--show" => {
                    options.open.get_or_insert_with(Default::default);
                }
                "--open-message" => {
                    options.open.get_or_insert_with(Default::default).message = Some(
                        args.next()
                            .context("--open-message 缺少邮件 ID")?
                            .into_string()
                            .map_err(|_| anyhow::anyhow!("邮件 ID 无效"))?,
                    )
                }
                "--account" => {
                    options.open.get_or_insert_with(Default::default).account = Some(
                        args.next()
                            .context("--account 缺少邮箱")?
                            .into_string()
                            .map_err(|_| anyhow::anyhow!("邮箱无效"))?,
                    )
                }
                "--no-daemon" => options.no_daemon = true,
                "--notification-open" => {
                    let link = args
                        .next()
                        .context("通知地址缺失")?
                        .into_string()
                        .map_err(|_| anyhow::anyhow!("通知地址无效"))?;
                    let target = crate::notifications::read_notification(&link)?;
                    options.config = Some(target.config);
                    options.open = Some(OpenRequest {
                        message: Some(target.message_id),
                        account: Some(target.account),
                    });
                }
                "--help" | "-h" => {
                    println!("简信：桌面托盘与本地 Web 服务\n启动后只有托盘，不提供原生桌面邮件应用。点击托盘在默认浏览器中管理邮箱和邮件。\n可选：--config <路径>、--show（打开浏览器）、--no-daemon");
                    std::process::exit(0);
                }
                "--version" | "-V" => {
                    println!("jianxin-desktop {}", env!("CARGO_PKG_VERSION"));
                    std::process::exit(0);
                }
                _ => bail!("不支持的启动参数：{value}"),
            }
        }
        Ok(options)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn routing_handles_dash_prefixed_ids_without_cli_commands() {
        let options = Options::from_args(
            [
                "--config",
                "/path with spaces/config.json",
                "--open-message",
                "--mail",
                "--account",
                "a@example.com",
            ]
            .map(Into::into),
        )
        .unwrap();
        assert_eq!(
            options.config.unwrap(),
            PathBuf::from("/path with spaces/config.json")
        );
        assert_eq!(
            options.open.unwrap(),
            OpenRequest {
                message: Some("--mail".into()),
                account: Some("a@example.com".into())
            }
        );
        for args in [vec!["tui"], vec!["sync"], vec!["--config"]] {
            assert!(Options::from_args(args.into_iter().map(Into::into)).is_err());
        }
    }
}
