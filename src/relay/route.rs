use std::collections::HashMap;

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::model::{Group, GroupItem, GroupMode};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteState {
    pub group_id:        i64,
    pub current_item_id: i64,
    pub affinity_until:  i64,
    pub cooldowns:       HashMap<i64, i64>,
    #[serde(skip)]
    pub affinity_armed:  bool,
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
            affinity_until: 0,
            cooldowns: HashMap::new(),
            affinity_armed: false,
        })
    }

    pub fn pick_group_item(&self, group: &Group) -> Option<GroupItem> {
        if group.mode == GroupMode::Manual {
            return group
                .items
                .iter()
                .find(|item| item.id == group.active_item_id)
                .cloned();
        }

        let mut entry = self.get_or_create_route_entry(group.id);
        let route = entry.value_mut();
        let now = chrono::Utc::now().timestamp_millis();

        if route.affinity_until <= now {
            route.affinity_until = 0;
        }

        // Within affinity period: stick to current item
        if route.current_item_id != 0
            && route.affinity_until > now
            && let Some(item) = group.items.iter().find(|i| i.id == route.current_item_id)
        {
            return Some(item.clone());
        }

        // Pick highest priority available item (group.items are pre-sorted by priority ASC)
        for item in &group.items {
            if let Some(&deadline) = route.cooldowns.get(&item.id)
                && deadline > now
            {
                continue;
            }

            if route.current_item_id != item.id {
                route.current_item_id = item.id;
                route.affinity_armed = true;
                self.notify_change(route);
            }
            return Some(item.clone());
        }

        // If all items are temporarily in cooldown, probe the one with the earliest expiring cooldown
        if let Some(item) = group
            .items
            .iter()
            .min_by_key(|item| route.cooldowns.get(&item.id).copied().unwrap_or(0))
        {
            if route.current_item_id != item.id {
                route.current_item_id = item.id;
                route.affinity_armed = true;
                self.notify_change(route);
            }
            return Some(item.clone());
        }

        None
    }

    pub fn report_success(&self, group_id: i64, item_id: i64, affinity_seconds: i64) {
        let mut entry = self.get_or_create_route_entry(group_id);
        let route = entry.value_mut();
        route.cooldowns.remove(&item_id);

        if route.affinity_armed && affinity_seconds > 0 {
            route.affinity_until = chrono::Utc::now().timestamp_millis() + affinity_seconds * 1000;
            route.affinity_armed = false;
        }
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
            route.affinity_until = 0;
            route.affinity_armed = false;
        }
        self.notify_change(route);
    }

    fn notify_change(&self, route: &RouteState) {
        let _ = self.tx.send(route.clone());
    }
}
