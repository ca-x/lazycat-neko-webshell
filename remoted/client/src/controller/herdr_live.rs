//! Refresh authoritative Herdr navigation independently of terminal output.
use super::*;

impl Actor {
    pub(super) async fn refresh_herdr(&mut self) {
        let grant = self.model.herdr_grant.clone();
        if grant.is_empty()
            || self.resource(&grant, "herdr").is_err()
            || self.pending_requests().any(|pending| {
                matches!(
                    pending,
                    Pending::Herdr { .. } | Pending::HerdrRefresh { .. }
                )
            })
            || self.outstanding.len() >= 2
            || self.requests.pending().next().is_some()
        {
            return;
        }
        if let Err(error) = self
            .request(
                Operation::Herdr {
                    grant: grant.clone(),
                    method: "session.snapshot".into(),
                    params: serde_json::json!({}),
                },
                Pending::HerdrRefresh { grant },
            )
            .await
        {
            self.model.notice = format!("Herdr 刷新失败：{error:#}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn live_refresh_updates_agents_and_tabs_without_navigation_or_busy_state() {
        let (updates, mut receiver) = mpsc::channel(32);
        let mut actor = Actor::new(updates);
        actor.model.phase = Phase::Ready;
        actor.model.permission = Some(Permission::ReadOnly);
        actor.model.herdr_grant = "herdr".into();
        actor.model.resources = vec![Resource {
            id: "herdr".into(),
            name: "Herdr".into(),
            backend: "herdr".into(),
            herdr: true,
            terminal: false,
            files: false,
        }];
        let (session, mut requests) = Session::observed();
        actor.session = Some(session.into());
        actor.refresh_herdr().await;
        actor.refresh_herdr().await;
        let request = requests.try_recv().expect("one live refresh");
        assert!(
            requests.try_recv().is_err(),
            "only one refresh may be in flight"
        );
        actor.publish().await;
        let Some(Update::Model(model)) = receiver.recv().await else {
            panic!("model");
        };
        assert!(
            !model.opening,
            "background refresh must not disable navigation"
        );
        let data = json!({"snapshot":{"tabs":[{"tab_id":"tab","workspace_id":"w","label":"new tab"}],"panes":[{"pane_id":"p","tab_id":"tab","agent":"codex","agent_status":"working"}]}});
        assert!(
            actor
                .response(Response {
                    id: request.id,
                    event: Event::Herdr { data: data.clone() }
                })
                .await
                .unwrap()
        );
        assert_eq!(actor.model.herdr_workspace.tabs[0].label, "new tab");
        assert_eq!(
            actor.model.herdr_workspace.agents[0].status.as_deref(),
            Some("working")
        );
        actor.refresh_herdr().await;
        let request = requests.try_recv().unwrap();
        assert!(
            !actor
                .response(Response {
                    id: request.id,
                    event: Event::Herdr { data }
                })
                .await
                .unwrap(),
            "unchanged snapshots must not rerender the page"
        );
    }
    #[tokio::test]
    async fn background_refresh_errors_remain_scoped_but_authorization_errors_stay_fatal() {
        for (code, fatal) in [("REQUEST_FAILED", false), ("AUTHORIZATION_FAILED", true)] {
            let (updates, _receiver) = mpsc::channel(8);
            let mut actor = Actor::new(updates);
            actor.model.phase = Phase::Ready;
            actor.outstanding.insert(
                1,
                Outstanding {
                    kind: Pending::HerdrRefresh {
                        grant: "herdr".into(),
                    },
                    expires: Instant::now() + Duration::from_secs(30),
                },
            );
            let result = actor
                .response(Response {
                    id: 1,
                    event: Event::Error {
                        code: code.into(),
                        message: "fixture".into(),
                        outcome_unknown: false,
                    },
                })
                .await;
            assert_eq!(result.is_err(), fatal);
            if !fatal {
                assert!(actor.model.notice.starts_with("Herdr 刷新失败："));
                actor.model.herdr_grant = "herdr".into();
                actor.outstanding.insert(
                    2,
                    Outstanding {
                        kind: Pending::HerdrRefresh {
                            grant: "herdr".into(),
                        },
                        expires: Instant::now() + Duration::from_secs(30),
                    },
                );
                assert!(
                    actor
                        .response(Response {
                            id: 2,
                            event: Event::Herdr {
                                data: json!({"snapshot":{}})
                            }
                        })
                        .await
                        .unwrap()
                );
                assert!(
                    actor.model.notice.is_empty(),
                    "a successful refresh clears its own stale warning"
                );
            }
        }
    }
}
