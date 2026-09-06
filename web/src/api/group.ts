import { computed, ref, watchEffect, toValue, type MaybeRefOrGetter } from 'vue';
import { useMutation, useQuery, useQueryClient } from '@tanstack/vue-query';
import { apiRequest, openSseStream } from './client';
import { groupListQueryOptions } from './queries';
import type { ChannelModel } from './channel';

// GroupMode 表示分组的手动或故障转移路由模式。
export type GroupMode = 'manual' | 'failover';

// GroupRelayConfig 保存分组 Relay 配置。
export interface GroupRelayConfig {
    member_max_attempts: number;
    member_retry_interval_seconds: number;
    member_non_stream_response_timeout_seconds: number;
    member_stream_first_event_timeout_seconds: number;
    member_cooldown_seconds: number;
}

// GroupItem 是分组内可手动选择或故障转移的渠道模型。
export interface GroupItem {
    id?: number;
    group_id?: number;
    channel_model_id: number;
    channel_model?: ChannelModel;
    priority: number;
}

// GroupRuntimeStatus 表示 Relay 当前进程内的实时路由状态。
export interface GroupRuntimeStatus {
    group_id: number;
    current_item_id: number;
    cooldowns: Record<number, number>;
}

// Group 是客户端模型名称对应的渠道分组。
export interface Group {
    id?: number;
    name: string;
    mode: GroupMode;
    active_item_id: number;
    relay_config: GroupRelayConfig;
    items?: GroupItem[];
    runtime?: GroupRuntimeStatus; // runtime 仅由 SSE 合并到前端缓存，不属于 GET 返回的持久化字段。
}

// GroupItemAddRequest 是待新增的分组项。
export interface GroupItemAddRequest {
    channel_model_id: number;
    priority: number;
}

// GroupItemUpdateRequest 是待更新展示和故障转移顺序的分组项。
export interface GroupItemUpdateRequest {
    id: number;
    priority: number;
}

// GroupUpdateRequest 是分组普通配置和成员变更。
export interface GroupUpdateRequest {
    id: number;
    name?: string;
    mode?: GroupMode;
    relay_config?: GroupRelayConfig;
    items_to_add?: GroupItemAddRequest[];
    items_to_update?: GroupItemUpdateRequest[];
    items_to_delete?: number[];
}

// useGroupList 获取全部分组，并由明确需要运行状态的页面控制是否建立实时连接。
export function useGroupList(
    enabled: MaybeRefOrGetter<boolean> = true,
    runtimeEnabled: MaybeRefOrGetter<boolean> = false,
) {
    // runtimeByGroup 保存 SSE 推送的进程内状态，不写回后端分组配置。
    const runtimeByGroup = ref<Map<number, GroupRuntimeStatus>>(new Map());

    const query = useQuery({
        ...groupListQueryOptions,
        enabled: () => toValue(enabled),
        refetchInterval: 30000,
        refetchOnMount: 'always',
    });

    watchEffect((onCleanup) => {
        const isEnabled = toValue(enabled);
        const isRuntimeEnabled = toValue(runtimeEnabled);
        const hasInitialData = query.data.value !== undefined;

        if (!isEnabled || !isRuntimeEnabled || !hasInitialData) return;

        const cancel = openSseStream(
            '/api/v1/group/runtime/stream',
            (_eventType, data) => {
                try {
                    const update = JSON.parse(data) as GroupRuntimeStatus;
                    const next = new Map(runtimeByGroup.value);
                    next.set(update.group_id, update);
                    runtimeByGroup.value = next;
                } catch {
                    return;
                }
            },
            () => {
                if (runtimeByGroup.value.size > 0) {
                    runtimeByGroup.value = new Map();
                }
            },
        );

        onCleanup(() => {
            cancel();
        });
    });

    // 未开启实时连接时不合并 runtime，避免连接关闭前推送的状态残留在列表上。
    const runtimeActive = computed(() => toValue(enabled) && toValue(runtimeEnabled));
    const groups = computed(() => query.data.value?.map((group) => {
        const runtime = !runtimeActive.value || group.id === undefined ? undefined : runtimeByGroup.value.get(group.id);
        return runtime ? { ...group, runtime } : group;
    }));

    return { ...query, data: groups };
}

// useCreateGroup 创建分组。
export function useCreateGroup() {
    const queryClient = useQueryClient();
    return useMutation({
        mutationFn: (data: Group) =>
            apiRequest<Group>('/api/v1/group/create', { method: 'POST', body: data }),
        onSuccess: () => queryClient.invalidateQueries({ queryKey: groupListQueryOptions.queryKey }),
    });
}

// useUpdateGroup 更新分组配置和成员。
export function useUpdateGroup() {
    const queryClient = useQueryClient();
    return useMutation({
        mutationFn: (data: GroupUpdateRequest) =>
            apiRequest<Group>('/api/v1/group/update', { method: 'POST', body: data }),
        onSuccess: () => queryClient.invalidateQueries({ queryKey: groupListQueryOptions.queryKey }),
    });
}

// useUpdateGroupActiveItem 手动切换或清空分组当前渠道。
export function useUpdateGroupActiveItem() {
    const queryClient = useQueryClient();
    return useMutation({
        mutationFn: ({ groupId, itemId }: { groupId: number; itemId: number }) =>
            apiRequest<Group>(`/api/v1/group/active/${groupId}`, {
                method: 'POST',
                body: { item_id: itemId },
            }),
        onSuccess: () => queryClient.invalidateQueries({ queryKey: groupListQueryOptions.queryKey }),
    });
}

// useDeleteGroup 删除分组。
export function useDeleteGroup() {
    const queryClient = useQueryClient();
    return useMutation({
        mutationFn: (id: number) =>
            apiRequest<null>(`/api/v1/group/delete/${id}`, { method: 'DELETE' }),
        onSuccess: () => queryClient.invalidateQueries({ queryKey: groupListQueryOptions.queryKey }),
    });
}
