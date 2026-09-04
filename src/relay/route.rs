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
            // 优先选择本次请求尚未尝试过的 active_item
            if let Some(item) = group
                .items
                .iter()
                .find(|item| item.id == group.active_item_id && !attempted_items.contains(&item.id))
            {
                return Some(item.clone());
            }
            // 若 active_item 已失败，顺位尝试该手动分组中其余尚未尝试的备用 item
            return group
                .items
                .iter()
                .find(|item| !attempted_items.contains(&item.id))
                .cloned();
        }

        let mut entry = self.get_or_create_route_entry(group.id);
        let route = entry.value_mut();
        let now = chrono::Utc::now().timestamp_millis();

        // 选择尚未尝试且不在冷却期的最高优先级可用渠道（items 已按 priority 升序预排序）
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

        // 若全部可用渠道均处于冷却中，从未尝试的渠道中选择最早冷却结束的一个进行探活
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
        // 仅当确实清除了冷却时才广播，避免无变化的重复事件
        if route.cooldowns.remove(&item_id).is_some() {
            self.notify_change(route);
        }
    }

    /// 分组删除后同步移除其路由状态，避免 get_snapshot 持续广播已删除分组的幽灵条目。
    pub fn remove_group(&self, group_id: i64) {
        self.routes.remove(&group_id);
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

        // 1. active_item_id 为 0（默认未指定）-> 回退选择第一个渠道项
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
            .expect("应能选中候选项");
        assert_eq!(picked.id, 10);

        // 2. active_item_id 指向不存在的项 -> 回退选择第一个渠道项
        let group_stale_active = Group {
            active_item_id: 999,
            ..group_unselected.clone()
        };
        let picked_stale = manager
            .pick_candidate_item(&group_stale_active, &HashSet::new())
            .expect("应能选中候选项");
        assert_eq!(picked_stale.id, 10);

        // 3. active_item_id 明确指定项 2 -> 选择项 2
        let group_selected_item2 = Group {
            active_item_id: 20,
            ..group_unselected.clone()
        };
        let picked2 = manager
            .pick_candidate_item(&group_selected_item2, &HashSet::new())
            .expect("应能选中候选项");
        assert_eq!(picked2.id, 20);

        // 4. 空渠道项列表 -> 返回 None
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
        // 首次选取: 项 101
        let first = manager.pick_candidate_item(&group, &attempted).unwrap();
        assert_eq!(first.id, 101);

        // 标记 101 已尝试
        attempted.insert(101);
        let second = manager.pick_candidate_item(&group, &attempted).unwrap();
        assert_eq!(second.id, 102);

        // 标记 102 已尝试 -> 候选耗尽返回 None
        attempted.insert(102);
        assert!(manager.pick_candidate_item(&group, &attempted).is_none());
    }
}
