use std::collections::{HashMap, HashSet};

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::model::{Group, GroupItem, GroupMode};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteState {
    pub group_id:        i64,
    pub current_item_id: i64,
    pub cooldowns:       HashMap<i64, i64>,
}

pub struct RouteManager {
    routes: DashMap<i64, RouteState>,
    tx:     broadcast::Sender<RouteState>,
}

impl RouteManager {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(128);
        Self {
            routes: DashMap::new(),
            tx,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RouteState> {
        self.tx.subscribe()
    }

    pub fn get_snapshot(&self) -> Vec<RouteState> {
        self.routes
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    fn get_or_create_route_entry<'a>(&'a self, group_id: i64) -> dashmap::mapref::one::RefMut<'a, i64, RouteState> {
        self.routes.entry(group_id).or_insert_with(|| RouteState {
            group_id,
            current_item_id: 0,
            cooldowns: HashMap::new(),
        })
    }

    pub fn pick_candidate_item(&self, group: &Group, attempted_items: &HashSet<i64>) -> Option<GroupItem> {
        if group.mode == GroupMode::Manual {
            // Prefer active_item if not yet attempted in this request
            if let Some(item) = group
                .items
                .iter()
                .find(|item| item.id == group.active_item_id && !attempted_items.contains(&item.id))
            {
                return Some(item.clone());
            }
            // Failover to other unattempted items in the manual group if active item failed
            return group
                .items
                .iter()
                .find(|item| !attempted_items.contains(&item.id))
                .cloned();
        }

        let mut entry = self.get_or_create_route_entry(group.id);
        let route = entry.value_mut();
        let now = chrono::Utc::now().timestamp_millis();

        // Pick highest priority available item not yet attempted in this request
        for item in &group.items {
            if attempted_items.contains(&item.id) {
                continue;
            }
            if let Some(&deadline) = route.cooldowns.get(&item.id)
                && deadline > now
            {
                continue;
            }

            if route.current_item_id != item.id {
                route.current_item_id = item.id;
                self.notify_change(route);
            }
            return Some(item.clone());
        }

        // If all available items are in cooldown, probe the unattempted one with earliest expiring cooldown
        if let Some(item) = group
            .items
            .iter()
            .filter(|item| !attempted_items.contains(&item.id))
            .min_by_key(|item| route.cooldowns.get(&item.id).copied().unwrap_or(0))
        {
            if route.current_item_id != item.id {
                route.current_item_id = item.id;
                self.notify_change(route);
            }
            return Some(item.clone());
        }

        None
    }

    pub fn report_success(&self, group_id: i64, item_id: i64) {
        let mut entry = self.get_or_create_route_entry(group_id);
        let route = entry.value_mut();
        route.cooldowns.remove(&item_id);
        self.notify_change(route);
    }

    pub fn report_failure(&self, group_id: i64, item_id: i64, cooldown_seconds: i64) {
        let mut entry = self.get_or_create_route_entry(group_id);
        let route = entry.value_mut();
        let now = chrono::Utc::now().timestamp_millis();
        route
            .cooldowns
            .insert(item_id, now + cooldown_seconds * 1000);

        if route.current_item_id == item_id {
            route.current_item_id = 0;
        }
        self.notify_change(route);
    }

    fn notify_change(&self, route: &RouteState) {
        let _ = self.tx.send(route.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manual_mode_fallback_to_first_item() {
        let manager = RouteManager::new();

        let item1 = GroupItem {
            id:               10,
            group_id:         1,
            channel_model_id: 101,
            channel_model:    None,
            priority:         1,
        };
        let item2 = GroupItem {
            id:               20,
            group_id:         1,
            channel_model_id: 102,
            channel_model:    None,
            priority:         2,
        };

        // 1. active_item_id is 0 (default unselected) -> falls back to first item
        let group_unselected = Group {
            id:             1,
            name:           "test-manual".to_string(),
            mode:           GroupMode::Manual,
            active_item_id: 0,
            relay_config:   Default::default(),
            items:          vec![item1.clone(), item2.clone()],
        };
        let picked = manager
            .pick_candidate_item(&group_unselected, &HashSet::new())
            .expect("should pick item");
        assert_eq!(picked.id, 10);

        // 2. active_item_id points to nonexistent item -> falls back to first item
        let group_stale_active = Group {
            active_item_id: 999,
            ..group_unselected.clone()
        };
        let picked_stale = manager
            .pick_candidate_item(&group_stale_active, &HashSet::new())
            .expect("should pick item");
        assert_eq!(picked_stale.id, 10);

        // 3. active_item_id is explicitly item 2 -> picks item 2
        let group_selected_item2 = Group {
            active_item_id: 20,
            ..group_unselected.clone()
        };
        let picked2 = manager
            .pick_candidate_item(&group_selected_item2, &HashSet::new())
            .expect("should pick item");
        assert_eq!(picked2.id, 20);

        // 4. empty items -> returns None
        let group_empty = Group {
            items: Vec::new(),
            ..group_unselected
        };
        assert!(
            manager
                .pick_candidate_item(&group_empty, &HashSet::new())
                .is_none()
        );
    }

    #[test]
    fn test_pick_candidate_item_excludes_attempted() {
        let manager = RouteManager::new();
        let group = Group {
            id:             1,
            name:           "test-failover".to_string(),
            mode:           GroupMode::Failover,
            active_item_id: 0,
            items:          vec![
                GroupItem {
                    id:               101,
                    group_id:         1,
                    channel_model_id: 1,
                    channel_model:    None,
                    priority:         1,
                },
                GroupItem {
                    id:               102,
                    group_id:         1,
                    channel_model_id: 2,
                    channel_model:    None,
                    priority:         2,
                },
            ],
            relay_config:   Default::default(),
        };

        let mut attempted = HashSet::new();
        // First pick: item 101
        let first = manager.pick_candidate_item(&group, &attempted).unwrap();
        assert_eq!(first.id, 101);

        // Mark 101 attempted
        attempted.insert(101);
        let second = manager.pick_candidate_item(&group, &attempted).unwrap();
        assert_eq!(second.id, 102);

        // Mark 102 attempted -> None
        attempted.insert(102);
        assert!(manager.pick_candidate_item(&group, &attempted).is_none());
    }
}
