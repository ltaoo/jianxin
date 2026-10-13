//! Explicit account-scoped synchronization, sharing the desktop polling worker.
use super::*;
use std::sync::Mutex;

#[derive(Default)]
pub(super) struct Refresher {
    pending: Mutex<HashSet<String>>,
}

struct Pending<'a> {
    refresher: &'a Refresher,
    account_id: String,
}

impl Drop for Pending<'_> {
    fn drop(&mut self) {
        self.refresher
            .pending
            .lock()
            .unwrap()
            .remove(&self.account_id);
    }
}

impl Refresher {
    pub(super) fn post(&self, app: &App, runtime: &dyn crate::RuntimeHooks, route: &str) -> Reply {
        let parsed = ammonia::url::Url::parse(&format!("http://localhost/{route}")).unwrap();
        let accounts = parsed
            .query_pairs()
            .filter(|(key, _)| key == "account")
            .map(|(_, value)| value.into_owned())
            .collect::<Vec<_>>();
        let Some(cfg) = accounts
            .first()
            .filter(|_| accounts.len() == 1)
            .and_then(|id| {
                app.account_configs()
                    .iter()
                    .find(|cfg| cfg.to_account().id == *id)
            })
        else {
            return Reply::json(400, serde_json::json!({"error":"请选择有效的邮箱"}));
        };
        let account_id = cfg.to_account().id;
        if !self.pending.lock().unwrap().insert(account_id.clone()) {
            return Reply::json(
                409,
                serde_json::json!({"error":"此邮箱正在刷新，请稍后重试"}),
            );
        }
        let _pending = Pending {
            refresher: self,
            account_id: account_id.clone(),
        };
        let result = (|| -> Result<()> {
            if runtime.refresh_account(&account_id)? {
                return Ok(());
            }
            let sync = (|| -> Result<()> {
                let provider = app.provider(cfg)?;
                let report = app.sync().sync_account(&*provider, &account_id, false)?;
                if let Some(failure) = report.failures.first() {
                    bail!("{}", failure.error);
                }
                Ok(())
            })();
            let error = sync.as_ref().err().map(ToString::to_string);
            app.record_sync_status(&account_id, error.as_deref());
            sync
        })();
        match result {
            Ok(()) => Reply::json(
                200,
                serde_json::json!({"account":account_id,"status":"synced"}),
            ),
            Err(error) => {
                eprintln!("refresh mailbox {account_id}: {error:#}");
                Reply::json(
                    502,
                    serde_json::json!({"error":"刷新邮箱失败，请检查邮箱状态后重试"}),
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[derive(Default)]
    struct Hooks {
        calls: Mutex<Vec<String>>,
        fail: AtomicBool,
    }
    impl crate::RuntimeHooks for Hooks {
        fn refresh_account(&self, account_id: &str) -> Result<bool> {
            self.calls.lock().unwrap().push(account_id.into());
            if self.fail.load(Ordering::SeqCst) {
                bail!("private-password from provider");
            }
            Ok(true)
        }
    }

    #[test]
    fn refresh_scopes_to_the_requested_mailbox_rejects_invalid_targets_and_allows_retry() {
        let config = serde_json::from_value(serde_json::json!({"accounts":[
            {"email":"first@example.com","auth":{"type":"password","password":"unused"}},
            {"email":"second@example.com","auth":{"type":"password","password":"unused"}}
        ]}))
        .unwrap();
        let app =
            App::with_database(config, jianxin_store::Database::open_in_memory().unwrap()).unwrap();
        let first = app.account_configs()[0].to_account().id;
        let second = app.account_configs()[1].to_account().id;
        let refresh = Refresher::default();
        let hooks = Hooks::default();
        let route = format!("api/mailboxes/refresh?account={second}");
        assert_eq!(refresh.post(&app, &hooks, &route).status, 200);
        assert_eq!(*hooks.calls.lock().unwrap(), vec![second.clone()]);
        for query in [
            String::new(),
            "account=".into(),
            "account=unknown".into(),
            format!("account={first}&account={second}"),
        ] {
            assert_eq!(
                refresh
                    .post(&app, &hooks, &format!("api/mailboxes/refresh?{query}"))
                    .status,
                400
            );
        }
        assert_eq!(hooks.calls.lock().unwrap().len(), 1);
        refresh.pending.lock().unwrap().insert(second.clone());
        assert_eq!(refresh.post(&app, &hooks, &route).status, 409);
        assert_eq!(hooks.calls.lock().unwrap().len(), 1);
        refresh.pending.lock().unwrap().remove(&second);
        hooks.fail.store(true, Ordering::SeqCst);
        let response = refresh.post(&app, &hooks, &route);
        assert_eq!(response.status, 502);
        assert!(!response.body.contains("private-password"));
        assert!(refresh.pending.lock().unwrap().is_empty());
        hooks.fail.store(false, Ordering::SeqCst);
        assert_eq!(refresh.post(&app, &hooks, &route).status, 200);
        assert_eq!(
            *hooks.calls.lock().unwrap(),
            vec![second.clone(), second.clone(), second]
        );
    }
}
