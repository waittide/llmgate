<script setup lang="ts">
import { ref, computed, watch, nextTick, useId, onMounted } from 'vue';
import { LuGripVertical, LuLayers, LuTrash2, LuX } from 'vue-icons-plus/lu';
import { VueDraggable } from 'vue-draggable-plus';
import { motion, AnimatePresence } from 'motion-v';
import { cn } from '@/lib/utils';
import { useTranslations } from '@/composables/i18n';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import type { Group } from '@/api/group';
import { getModelIcon } from './model-icons';
import MemberStatus from './MemberStatus.vue';

export interface SelectedMember {
    id: string;
    channel_model_id: number;
    name: string;
    enabled: boolean;
    channel_id: number;
    channel_name: string;
    item_id?: number;
}

export interface MemberListProps {
    members: SelectedMember[];
    activeItemId?: number;
    group?: Group; // group 提供当前模式和成员运行状态。
    now?: number; // now 是页面共享的当前 Unix 毫秒时间，仅展示运行态时需要。
    autoScrollOnAdd?: boolean;
    removingIds?: Set<string>;
    showConfirmDelete?: boolean;
    layoutScope?: string;
}

const props = withDefaults(defineProps<MemberListProps>(), {
    activeItemId: undefined,
    group: undefined,
    now: 0,
    autoScrollOnAdd: true,
    removingIds: () => new Set<string>(),
    showConfirmDelete: true,
    layoutScope: undefined,
});

const emit = defineEmits<{
    (e: 'update:members', members: SelectedMember[]): void;
    (e: 'reorder', members: SelectedMember[]): void;
    (e: 'remove', id: string): void;
    (e: 'activate', itemId: number): void;
    (e: 'dragStart'): void;
    (e: 'drop', members: SelectedMember[]): void;
    (e: 'dragFinish'): void;
}>();

const { t } = useTranslations('group');
const internalLayoutScope = useId();
const effectiveLayoutScope = computed(() => props.layoutScope ?? internalLayoutScope);

const scrollContainerRef = ref<HTMLDivElement | null>(null);
const localMembers = ref<SelectedMember[]>([...props.members]);
const isDragging = ref(false);
const prevMemberCount = ref(props.members.length);
const hasMounted = ref(false);
const confirmDeleteMap = ref<Record<string, boolean>>({});

const visibleCount = computed(() => props.members.filter((m) => !props.removingIds.has(m.id)).length);
const isEmpty = computed(() => visibleCount.value === 0);

watch(
    () => props.members,
    (next) => {
        if (!isDragging.value) {
            localMembers.value = [...next];
        }
    },
    { deep: true },
);

onMounted(() => {
    hasMounted.value = true;
    prevMemberCount.value = props.members.length;
});

watch(
    () => props.members.length,
    (nextLen) => {
        if (!hasMounted.value) return;

        if (!props.autoScrollOnAdd) {
            prevMemberCount.value = nextLen;
            return;
        }

        const hasNewMember = nextLen > prevMemberCount.value;
        if (hasNewMember) {
            nextTick(() => {
                const el = scrollContainerRef.value;
                if (el) {
                    el.scrollTo({ top: el.scrollHeight, behavior: 'smooth' });
                }
            });
        }
        prevMemberCount.value = nextLen;
    },
);

function handleDragStart() {
    isDragging.value = true;
    emit('dragStart');
}

function handleDragEnd(evt: { oldIndex?: number; newIndex?: number }) {
    isDragging.value = false;
    emit('update:members', [...localMembers.value]);
    emit('reorder', [...localMembers.value]);
    if (evt.oldIndex !== undefined && evt.newIndex !== undefined && evt.oldIndex !== evt.newIndex) {
        emit('drop', [...localMembers.value]);
    }
    emit('dragFinish');
}

function handleItemClick(member: SelectedMember) {
    if (member.item_id !== undefined) {
        emit('activate', member.item_id);
    }
}

function handleItemKeyDown(event: KeyboardEvent, member: SelectedMember) {
    if (event.target !== event.currentTarget || member.item_id === undefined || (event.key !== 'Enter' && event.key !== ' ')) {
        return;
    }
    event.preventDefault();
    emit('activate', member.item_id);
}

function handleRemoveClick(id: string) {
    if (props.showConfirmDelete) {
        confirmDeleteMap.value[id] = true;
    } else {
        emit('remove', id);
    }
}

function handleCancelDelete(id: string) {
    confirmDeleteMap.value[id] = false;
}

function handleConfirmDelete(id: string) {
    confirmDeleteMap.value[id] = false;
    emit('remove', id);
}
</script>

<template>
    <div class="relative h-full min-h-0">
        <!-- 空状态提示 -->
        <div
            :class="cn(
                'absolute inset-0 flex flex-col items-center justify-center gap-2 text-muted-foreground transition-opacity duration-200 ease-out',
                isEmpty ? 'opacity-100' : 'opacity-0 pointer-events-none'
            )"
        >
            <LuLayers class="size-10 opacity-40" />
            <span class="text-sm">{{ t('card.empty') }}</span>
        </div>

        <!-- 列表滚动容器 -->
        <div
            ref="scrollContainerRef"
            :class="cn(
                'h-full overflow-y-auto transition-opacity duration-200',
                isEmpty ? 'opacity-0' : 'opacity-100'
            )"
        >
            <VueDraggable
                v-model="localMembers"
                :animation="150"
                handle=".drag-handle"
                ghost-class="opacity-50"
                :disabled="false"
                class="p-2 flex flex-col space-y-1.5"
                @start="handleDragStart"
                @end="handleDragEnd"
            >
                <div
                    v-for="member in localMembers"
                    :key="member.id"
                    :class="cn(
                        'rounded-lg grid transition-[grid-template-rows] duration-200',
                        removingIds.has(member.id) ? 'grid-rows-[0fr]' : 'grid-rows-[1fr]'
                    )"
                >
                    <div
                        :class="cn(
                            'flex items-center gap-2 rounded-lg bg-background px-2.5 py-2 select-none transition-[background-color,opacity] duration-200 relative overflow-hidden',
                            removingIds.has(member.id) && 'opacity-0',
                            member.enabled === false && 'opacity-60 grayscale',
                            group?.mode === 'manual' && member.item_id !== undefined && 'cursor-pointer'
                        )"
                        :role="group?.mode === 'manual' && member.item_id !== undefined ? 'button' : undefined"
                        :tabindex="group?.mode === 'manual' && member.item_id !== undefined ? 0 : undefined"
                        :title="group?.mode === 'manual' && member.item_id !== undefined && member.item_id !== activeItemId ? t('card.activate') : undefined"
                        @click="handleItemClick(member)"
                        @keydown="handleItemKeyDown($event, member)"
                    >
                        <!-- 拖拽手柄 -->
                        <div
                            :class="cn(
                                'drag-handle p-0.5 rounded touch-none transition-colors',
                                member.enabled === false
                                    ? 'cursor-grab active:cursor-grabbing hover:bg-muted/60'
                                    : 'cursor-grab active:cursor-grabbing hover:bg-muted'
                            )"
                            @click.stop
                        >
                            <LuGripVertical class="size-3.5 text-muted-foreground" />
                        </div>

                        <!-- 模型图标 -->
                        <span :class="cn(member.enabled === false && 'opacity-70')">
                            <component
                                :is="getModelIcon(member.name).Icon"
                                aria-hidden="true"
                                :class="getModelIcon(member.name).className"
                                :style="{ width: '18px', height: '18px' }"
                            />
                        </span>

                        <!-- 模型与渠道名称 -->
                        <div class="flex flex-col min-w-0 flex-1">
                            <Tooltip>
                                <TooltipTrigger as-child>
                                    <span
                                        :class="cn(
                                            'inline-block w-fit max-w-full text-sm font-medium truncate leading-tight',
                                            member.enabled === false && 'text-muted-foreground'
                                        )"
                                    >
                                        {{ member.name }}
                                    </span>
                                </TooltipTrigger>
                                <TooltipContent :key="member.name" side="top" :side-offset="10" align="center">
                                    {{ member.name }}
                                </TooltipContent>
                            </Tooltip>
                            <span class="inline-block w-fit max-w-full text-[10px] text-muted-foreground truncate leading-tight">
                                {{ member.channel_name }}
                            </span>
                        </div>

                        <!-- 成员状态（冷却或活动中标记） -->
                        <MemberStatus
                            v-if="group"
                            :group="group"
                            :item-id="member.item_id"
                            :now="now"
                            :disabled="member.enabled === false"
                            :active="member.item_id === activeItemId"
                            active-class-name="p-1"
                        />

                        <!-- 删除按钮 -->
                        <motion.button
                            v-if="!showConfirmDelete || !confirmDeleteMap[member.id]"
                            :layout-id="`delete-btn-member-${effectiveLayoutScope}-${member.id}`"
                            type="button"
                            class="p-1 rounded hover:bg-destructive/10 hover:text-destructive transition-colors"
                            :transition="{ duration: 0.15 }"
                            style="pointer-events: auto"
                            @click.stop="handleRemoveClick(member.id)"
                        >
                            <LuX class="size-3" />
                        </motion.button>

                        <!-- 删除确认遮罩 -->
                        <AnimatePresence>
                            <motion.div
                                v-if="showConfirmDelete && confirmDeleteMap[member.id]"
                                :layout-id="`delete-btn-member-${effectiveLayoutScope}-${member.id}`"
                                class="absolute inset-0 flex items-center justify-center gap-2 bg-destructive p-1.5 rounded-lg z-10"
                                :transition="{ type: 'spring', stiffness: 400, damping: 30 }"
                                @click.stop
                            >
                                <button
                                    type="button"
                                    class="flex h-6 w-6 items-center justify-center rounded-md bg-destructive-foreground/20 text-destructive-foreground transition-all hover:bg-destructive-foreground/30 active:scale-95"
                                    @click.stop="handleCancelDelete(member.id)"
                                >
                                    <LuX class="size-3" />
                                </button>
                                <button
                                    type="button"
                                    class="flex-1 h-6 flex items-center justify-center gap-1.5 rounded-md bg-destructive-foreground text-destructive text-xs font-semibold transition-all hover:bg-destructive-foreground/90 active:scale-[0.98]"
                                    @click.stop="handleConfirmDelete(member.id)"
                                >
                                    <LuTrash2 class="size-3" />
                                </button>
                            </motion.div>
                        </AnimatePresence>
                    </div>
                </div>
            </VueDraggable>
        </div>
    </div>
</template>
