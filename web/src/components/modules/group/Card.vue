<script setup lang="ts">
import { ref, computed, watch } from 'vue';
import { LuPencil, LuTrash2, LuX } from 'vue-icons-plus/lu';
import { motion, AnimatePresence } from 'motion-v';
import {
    type Group,
    type GroupUpdateRequest,
    useDeleteGroup,
    useUpdateGroup,
    useUpdateGroupActiveItem,
} from '@/api/group';
import { useChannelList } from '@/api/channel';
import { useTranslations } from '@/composables/i18n';
import { toast } from '@/composables/toast';
import CopyIconButton from '@/components/common/CopyButton.vue';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import {
    MorphingDialog,
    MorphingDialogClose,
    MorphingDialogContainer,
    MorphingDialogContent,
    MorphingDialogDescription,
    MorphingDialogTitle,
    MorphingDialogTrigger,
} from '@/components/ui/morphing-dialog';
import MemberList, { type SelectedMember } from './ItemList.vue';
import GroupEditor, { type GroupEditorValues } from './Editor.vue';

export interface GroupCardProps {
    group: Group;
    now: number;
}

const props = defineProps<GroupCardProps>();

const { t } = useTranslations('group');
const updateGroup = useUpdateGroup();
const updateActiveItem = useUpdateGroupActiveItem();
const deleteGroup = useDeleteGroup();
const { data: channelsData } = useChannelList();

const confirmDelete = ref(false);
const members = ref<SelectedMember[]>([]);
const isDragging = ref(false);
const editDialogOpen = ref(false);

const channelByModelID = computed(() => {
    const map = new Map<number, { channel_name: string; enabled: boolean }>();
    const list = channelsData.value ?? [];
    list.forEach(({ raw: channel }) => {
        channel.models.forEach((channelModel) => {
            map.set(channelModel.id, { channel_name: channel.name, enabled: channel.enabled });
        });
    });
    return map;
});

const displayMembers = computed<SelectedMember[]>(() =>
    (props.group.items || []).map((item) => {
        const channelModel = item.channel_model;
        const channel = channelByModelID.value.get(item.channel_model_id);
        return {
            id: String(item.channel_model_id),
            channel_model_id: item.channel_model_id,
            name: channelModel?.name ?? `Model ${item.channel_model_id}`,
            enabled: channel?.enabled ?? true,
            channel_id: channelModel?.channel_id ?? 0,
            channel_name: channel?.channel_name ?? 'Unknown channel',
            item_id: item.id,
        };
    })
);

watch(
    displayMembers,
    (next) => {
        if (!isDragging.value) {
            members.value = [...next];
        }
    },
    { immediate: true },
);

const priorityByItemId = computed(() => {
    const map = new Map<number, number>();
    (props.group.items || []).forEach((item) => {
        if (item.id !== undefined) {
            map.set(item.id, item.priority);
        }
    });
    return map;
});

function onSuccess() {
    toast.success(t('toast.updated'));
}

function onError(error: Error) {
    toast.error(t('toast.updateFailed') + (error.message ? `: ${error.message}` : ''));
}

function handleDragStart() {
    isDragging.value = true;
}

function handleDragFinish() {
    isDragging.value = false;
}

function handleDropReorder(nextMembers: SelectedMember[]) {
    if (!props.group.id) return;

    const itemsToUpdate = nextMembers
        .map((m, i) => ({ member: m, newPriority: i + 1 }))
        .filter(({ member, newPriority }) => {
            if (!member.item_id) return false;
            const origPriority = priorityByItemId.value.get(member.item_id);
            return origPriority !== undefined && origPriority !== newPriority;
        })
        .map(({ member, newPriority }) => ({ id: member.item_id!, priority: newPriority }));

    if (itemsToUpdate.length > 0) {
        updateGroup.mutate(
            { id: props.group.id, items_to_update: itemsToUpdate },
            { onSuccess, onError },
        );
    }
}

function handleRemoveMember(id: string) {
    if (!props.group.id) return;
    const member = members.value.find((m) => m.id === id);
    if (member?.item_id !== undefined) {
        updateGroup.mutate(
            { id: props.group.id, items_to_delete: [member.item_id] },
            { onSuccess, onError },
        );
    }
}

function handleActivate(itemId: number) {
    if (
        !props.group.id ||
        props.group.mode !== 'manual' ||
        itemId === props.group.active_item_id ||
        updateActiveItem.isPending.value
    ) {
        return;
    }
    updateActiveItem.mutate({ groupId: props.group.id, itemId }, { onSuccess, onError });
}

function handleDelete() {
    if (!props.group.id) return;
    deleteGroup.mutate(props.group.id, {
        onSuccess: () => {
            confirmDelete.value = false;
            toast.success(t('toast.deleted'));
        },
    });
}

function handleSubmitEdit(values: GroupEditorValues) {
    if (!props.group.id) return;

    const originalById = new Map<number, number>();
    const originalIds = new Set<number>();
    (props.group.items || []).forEach((it) => {
        if (typeof it.id === 'number') {
            originalIds.add(it.id);
            originalById.set(it.id, it.priority);
        }
    });

    const newIds = new Set<number>();
    values.members.forEach((m) => {
        if (typeof m.item_id === 'number') {
            newIds.add(m.item_id);
        }
    });

    const items_to_delete = Array.from(originalIds).filter((id) => !newIds.has(id));

    const items_to_add = values.members
        .map((m, idx) => ({ m, priority: idx + 1 }))
        .filter(({ m }) => typeof m.item_id !== 'number')
        .map(({ m, priority }) => ({
            channel_model_id: m.channel_model_id,
            priority,
        }));

    const items_to_update = values.members
        .map((m, idx) => ({ m, priority: idx + 1 }))
        .filter(({ m }) => typeof m.item_id === 'number')
        .map(({ m, priority }) => {
            const id = m.item_id!;
            const originalPriority = originalById.get(id);
            if (originalPriority === undefined || originalPriority === priority) return null;
            return { id, priority };
        })
        .filter((item): item is { id: number; priority: number } => item !== null);

    const payload: GroupUpdateRequest = { id: props.group.id };

    if (values.name !== props.group.name) payload.name = values.name;
    if (values.mode !== props.group.mode) payload.mode = values.mode;

    if (
        values.relay_config.member_max_attempts !== props.group.relay_config.member_max_attempts ||
        values.relay_config.member_retry_interval_seconds !== props.group.relay_config.member_retry_interval_seconds ||
        values.relay_config.member_non_stream_response_timeout_seconds !== props.group.relay_config.member_non_stream_response_timeout_seconds ||
        values.relay_config.member_stream_first_event_timeout_seconds !== props.group.relay_config.member_stream_first_event_timeout_seconds ||
        values.relay_config.member_cooldown_seconds !== props.group.relay_config.member_cooldown_seconds
    ) {
        payload.relay_config = values.relay_config;
    }

    if (items_to_add.length > 0) payload.items_to_add = items_to_add;
    if (items_to_update.length > 0) payload.items_to_update = items_to_update;
    if (items_to_delete.length > 0) payload.items_to_delete = items_to_delete;

    if (Object.keys(payload).length === 1) {
        editDialogOpen.value = false;
        return;
    }

    updateGroup.mutate(payload, {
        onSuccess: () => {
            onSuccess();
            editDialogOpen.value = false;
        },
        onError,
    });
}
</script>

<template>
    <article class="flex flex-col rounded-3xl border border-border bg-card text-card-foreground p-4">
        <header class="flex items-start justify-between mb-3 relative overflow-visible rounded-xl -mx-1 px-1 -my-1 py-1">
            <div class="relative flex-1 mr-2 min-w-0 group/title">
                <Tooltip>
                    <TooltipTrigger as-child>
                        <h3 class="text-lg font-bold truncate">{{ group.name }}</h3>
                    </TooltipTrigger>
                    <TooltipContent :key="group.name" side="top" :side-offset="10" align="center">
                        {{ group.name }}
                    </TooltipContent>
                </Tooltip>
            </div>

            <div class="flex items-center gap-1 shrink-0">
                <!-- 编辑弹窗 -->
                <MorphingDialog v-model:open="editDialogOpen">
                    <Tooltip>
                        <TooltipTrigger as-child>
                            <MorphingDialogTrigger class="p-1.5 rounded-lg transition-colors hover:bg-muted text-muted-foreground hover:text-foreground inline-flex items-center justify-center">
                                <LuPencil class="size-4" />
                            </MorphingDialogTrigger>
                        </TooltipTrigger>
                        <TooltipContent side="top" :side-offset="10" align="center">
                            {{ t('detail.actions.edit') }}
                        </TooltipContent>
                    </Tooltip>
                    <MorphingDialogContainer>
                        <MorphingDialogContent class="relative w-screen max-w-full md:max-w-4xl bg-card text-card-foreground px-6 py-4 rounded-3xl h-[calc(100vh-2rem)] flex flex-col overflow-hidden">
                            <MorphingDialogTitle class="shrink-0">
                                <header class="mb-3 flex items-center justify-between">
                                    <h2 class="text-2xl font-bold text-card-foreground">
                                        {{ t('detail.actions.edit') }}
                                    </h2>
                                    <MorphingDialogClose class="relative right-0 top-0" />
                                </header>
                            </MorphingDialogTitle>
                            <MorphingDialogDescription class="flex-1 min-h-0 overflow-hidden">
                                <GroupEditor
                                    :key="`edit-group-${group.id}`"
                                    :initial="{
                                        name: group.name,
                                        mode: group.mode,
                                        relay_config: group.relay_config,
                                        members: displayMembers,
                                    }"
                                    :submit-text="t('detail.actions.save')"
                                    :submitting-text="t('create.submitting')"
                                    :is-submitting="updateGroup.isPending.value"
                                    @cancel="editDialogOpen = false"
                                    @submit="handleSubmitEdit"
                                />
                            </MorphingDialogDescription>
                        </MorphingDialogContent>
                    </MorphingDialogContainer>
                </MorphingDialog>

                <!-- 复制名称 -->
                <Tooltip>
                    <TooltipTrigger as-child>
                        <span class="inline-flex">
                            <CopyIconButton
                                :text="group.name"
                                class-name="p-1.5 rounded-lg transition-colors hover:bg-muted text-muted-foreground hover:text-foreground"
                                copy-icon-class-name="size-4"
                                check-icon-class-name="size-4 text-primary"
                            />
                        </span>
                    </TooltipTrigger>
                    <TooltipContent side="top" :side-offset="10" align="center">
                        {{ t('detail.actions.copyName') }}
                    </TooltipContent>
                </Tooltip>

                <!-- 删除按钮 -->
                <Tooltip v-if="!confirmDelete">
                    <TooltipTrigger as-child>
                        <motion.button
                            :layout-id="`delete-btn-group-${group.id}`"
                            type="button"
                            class="p-1.5 rounded-lg hover:bg-destructive/10 text-muted-foreground hover:text-destructive transition-colors"
                            @click="confirmDelete = true"
                        >
                            <LuTrash2 class="size-4" />
                        </motion.button>
                    </TooltipTrigger>
                    <TooltipContent side="top" :side-offset="10" align="center">
                        {{ t('detail.actions.delete') }}
                    </TooltipContent>
                </Tooltip>
            </div>

            <!-- 删除确认遮罩 -->
            <AnimatePresence>
                <motion.div
                    v-if="confirmDelete"
                    :layout-id="`delete-btn-group-${group.id}`"
                    class="absolute inset-0 flex items-center justify-center gap-2 bg-destructive p-2 rounded-xl z-10"
                    :transition="{ type: 'spring', stiffness: 400, damping: 30 }"
                >
                    <button
                        type="button"
                        class="flex h-7 w-7 items-center justify-center rounded-lg bg-destructive-foreground/20 text-destructive-foreground transition-all hover:bg-destructive-foreground/30 active:scale-95"
                        @click="confirmDelete = false"
                    >
                        <LuX class="size-4" />
                    </button>
                    <button
                        type="button"
                        :disabled="deleteGroup.isPending.value"
                        class="flex-1 h-7 flex items-center justify-center gap-2 rounded-lg bg-destructive-foreground text-destructive text-sm font-semibold transition-all hover:bg-destructive-foreground/90 active:scale-[0.98] disabled:opacity-50 disabled:cursor-not-allowed"
                        @click="handleDelete"
                    >
                        <LuTrash2 class="size-3.5" />
                        {{ t('detail.actions.confirmDelete') }}
                    </button>
                </motion.div>
            </AnimatePresence>
        </header>

        <section class="rounded-xl border border-border/50 bg-muted/30 overflow-hidden relative h-101">
            <MemberList
                :members="members"
                :active-item-id="group.mode === 'failover' ? group.runtime?.current_item_id : group.active_item_id"
                :group="group"
                :now="now"
                :auto-scroll-on-add="false"
                :layout-scope="`card-${group.id ?? 'unknown'}`"
                @reorder="members = $event"
                @remove="handleRemoveMember"
                @activate="group.mode === 'manual' ? handleActivate($event) : undefined"
                @drag-start="handleDragStart"
                @drop="handleDropReorder"
                @drag-finish="handleDragFinish"
            />
        </section>
    </article>
</template>
