<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watchEffect } from 'vue';
import {
    LuAlertCircle,
    LuArrowRight,
    LuLoader2,
    LuSquare,
} from 'vue-icons-plus/lu';
import { useTranslations } from '@/composables/i18n';
import { toast } from '@/composables/toast';
import { cn } from '@/lib/utils';
import {
    type RelayLogOverview,
    type RoundLog,
    useLogDetail,
    useLogRequestBody,
    useLogResponseBody,
    useStopRound,
} from '@/api/log';
import { useGroupList, useUpdateGroupActiveItem, type GroupItem } from '@/api/group';
import { useChannelList } from '@/api/channel';
import { Badge } from '@/components/ui/badge';
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs';
import CopyButton from '@/components/common/CopyButton.vue';
import {
    MorphingDialogContent,
    MorphingDialogClose,
    MorphingDialogTitle,
    MorphingDialogDescription,
} from '@/components/ui/morphing-dialog';
import { getModelIcon } from './model-icons';
import HttpMeta from './HttpMeta.vue';
import JsonContent from './JsonContent.vue';
import LogMetrics from './LogMetrics.vue';
import MemberStatus from './MemberStatus.vue';

interface ObservedRound {
    round: number;
    channel: string;
    error: string;
    sending: boolean;
}

interface Props {
    log: RelayLogOverview;
    now: number;
}

const props = defineProps<Props>();

const { t } = useTranslations('log.card');
const { t: statusT } = useTranslations('log.status');

const leftTab = ref<'request' | 'group'>('group');
const rounds = ref<ObservedRound[]>([]);
const observedRoundKey = ref('');
const detailReady = ref(false);
const switchingItemId = ref<number | null>(null);

const requestBody = useLogRequestBody(
    computed(() => props.log.id),
    computed(() => props.log.started_at),
    computed(() => detailReady.value && leftTab.value === 'request'),
);

const responseBody = useLogResponseBody(
    computed(() => props.log.id),
    computed(() => props.log.started_at),
    computed(() => detailReady.value && props.log.status === 'success'),
);

const detail = useLogDetail(
    computed(() => props.log.id),
    computed(() => detailReady.value),
);

const { data: groupsData } = useGroupList(
    computed(() => detailReady.value),
    computed(() => detailReady.value),
);
const groups = computed(() => groupsData.value ?? []);

const { data: channelsData } = useChannelList(computed(() => detailReady.value));
const channels = computed(() => channelsData.value ?? []);

const updateActiveItem = useUpdateGroupActiveItem();
const stopRound = useStopRound();

const channelNameByModelID = computed(() => {
    const map = new Map<number, string>();
    channels.value.forEach(({ raw: channel }) => {
        channel.models.forEach((channelModel) => map.set(channelModel.id, channel.name));
    });
    return map;
});

const persistentRounds = computed<RoundLog[]>(() => {
    if (props.log.rounds && props.log.rounds.length > 0) return props.log.rounds;
    if (detail.data.value?.rounds && detail.data.value.rounds.length > 0) return detail.data.value.rounds;
    if (detail.data.value?.rounds_detail) {
        try {
            return JSON.parse(detail.data.value.rounds_detail) as RoundLog[];
        } catch {
            return [];
        }
    }
    return [];
});

const selectedRoundNum = ref<number | null>(null);

const activeRound = computed(() => {
    if (persistentRounds.value.length === 0) return null;
    if (selectedRoundNum.value !== null) {
        return persistentRounds.value.find((r) => r.round === selectedRoundNum.value) || persistentRounds.value[persistentRounds.value.length - 1];
    }
    return persistentRounds.value[persistentRounds.value.length - 1];
});

const actualModel = computed(() => activeRound.value?.model_name || props.log.target_model || props.log.model);
const actualChannel = computed(() => activeRound.value?.channel_name || props.log.target_channel);
const modelIconInfo = computed(() => getModelIcon(actualModel.value));
const errorText = computed(() => activeRound.value?.error ?? props.log.error ?? '');
const requestFailed = computed(() => props.log.status === 'failed' || props.log.status === 'canceled');
const responseCommitted = computed(() => props.log.status === 'committed');
const showRounds = computed(() => props.log.status === 'running' && rounds.value.length > 0);
const activeGroup = computed(() => groups.value.find((group) => group.name.toLowerCase() === props.log.model?.toLowerCase()));
const isWaitingForSelection = computed(() => props.log.status === 'running' && !props.log.sending && activeGroup.value?.mode === 'manual' && activeGroup.value.active_item_id === 0);

onMounted(() => {
    const timer = window.setTimeout(() => {
        detailReady.value = true;
    }, 600);
    onUnmounted(() => window.clearTimeout(timer));
});

watchEffect(() => {
    const roundKey = props.log.round === 0 ? '' : `${props.log.round}:${props.log.target_channel}:${props.log.sending}:${props.log.error ?? ''}`;
    if (roundKey !== '' && roundKey !== observedRoundKey.value) {
        observedRoundKey.value = roundKey;
        if (!props.log.sending && rounds.value.every((item) => item.round !== props.log.round)) {
            return;
        }
        rounds.value = [
            { round: props.log.round, channel: props.log.target_channel, error: props.log.error ?? '', sending: props.log.sending },
            ...rounds.value.filter((item) => item.round !== props.log.round),
        ];
    }
});

function formatMilliseconds(value: number) {
    const milliseconds = Math.max(0, value);
    if (milliseconds < 1000) return `${Math.round(milliseconds)}ms`;
    return `${(milliseconds / 1000).toFixed(2)}s`;
}

function getItemChannelName(item: GroupItem): string {
    if (!item.channel_model) return '-';
    return (
        channelNameByModelID.value.get(item.channel_model.id) ??
        channels.value.find((c) => c.raw.id === item.channel_model?.channel_id)?.raw.name ??
        `#${item.channel_model.channel_id}`
    );
}

function getItemIcon(item: GroupItem) {
    const modelName = item.channel_model?.name ?? '';
    return getModelIcon(modelName);
}

function itemCurrent(item: GroupItem): boolean {
    if (!activeGroup.value) return false;
    const modelName = item.channel_model?.name ?? '';
    const channelName = getItemChannelName(item);
    const itemActive = item.id === activeGroup.value.active_item_id;
    const itemSwitching = item.id === switchingItemId.value;
    const isUsedByLog = Boolean(
        actualChannel.value &&
        (channelName === actualChannel.value || (!channelNameByModelID.value.size && (!channelName || channelName === '-'))) &&
        (!actualModel.value || modelName === actualModel.value)
    );

    if (switchingItemId.value !== null) {
        return itemSwitching;
    }
    if (isUsedByLog) {
        return true;
    }
    if (props.log.status === 'running') {
        return activeGroup.value.mode === 'failover'
            ? activeGroup.value.runtime?.current_item_id === item.id
            : itemActive;
    }
    return false;
}

async function handleSwitchItem(item: GroupItem) {
    if (!activeGroup.value?.id || item.id === undefined || activeGroup.value.mode === 'failover') return;
    const itemActive = item.id === activeGroup.value.active_item_id;
    switchingItemId.value = item.id;
    try {
        await updateActiveItem.mutateAsync({ groupId: activeGroup.value.id, itemId: itemActive ? 0 : item.id });
        if (props.log.sending) {
            await stopRound.mutateAsync({ requestId: props.log.id, round: props.log.round });
        }
        toast.success(itemActive ? t('channelCleared') : t('channelChanged'));
    } catch (cause) {
        const message = cause instanceof Error ? `${t('channelChangeFailed')}: ${cause.message}` : t('channelChangeFailed');
        toast.error(message);
    } finally {
        switchingItemId.value = null;
    }
}

async function handleStopRound() {
    try {
        await stopRound.mutateAsync({ requestId: props.log.id, round: props.log.round });
    } catch (cause) {
        const message = cause instanceof Error ? `${t('stopFailed')}: ${cause.message}` : t('stopFailed');
        toast.error(message);
    }
}
</script>

<template>
    <MorphingDialogContent class="relative w-[calc(100vw-2rem)] md:w-[80vw] bg-card text-card-foreground px-6 py-4 rounded-3xl h-[calc(100vh-2rem)] flex flex-col overflow-hidden">
        <MorphingDialogClose class="top-4 right-5 text-muted-foreground hover:text-foreground transition-colors" />
        <MorphingDialogTitle class="flex items-center gap-2 mb-3 text-sm">
            <component
                :is="modelIconInfo.Icon"
                aria-hidden="true"
                :class="modelIconInfo.className"
                width="28"
                height="28"
            />
            <span class="font-semibold text-card-foreground">{{ log.model || t('unknownModel') }}</span>
            <LuLoader2
                v-if="log.status === 'running' || responseCommitted"
                class="size-3.5 animate-spin text-muted-foreground/50"
            />
            <LuArrowRight
                v-else
                class="size-3.5 text-muted-foreground/50"
            />
            <Badge
                variant="secondary"
                class="text-xs px-1.5 py-0"
                :style="{ backgroundColor: `${modelIconInfo.color}15`, color: modelIconInfo.color }"
            >
                {{ actualChannel || '-' }}
            </Badge>
            <span class="text-muted-foreground">{{ actualModel }}</span>
            <Badge
                v-if="persistentRounds.length > 1"
                variant="outline"
                class="text-[11px] ml-auto"
            >
                {{ t('roundIndex', { index: activeRound?.round ?? log.round }) }} / {{ persistentRounds.length }}
            </Badge>
        </MorphingDialogTitle>

        <MorphingDialogDescription class="flex-1 min-h-0 flex flex-col gap-2">
            <!-- 轮次切换条 -->
            <div
                v-if="persistentRounds.length > 1"
                class="flex items-center gap-1.5 overflow-x-auto pb-1 shrink-0"
            >
                <span class="text-xs font-semibold text-muted-foreground mr-1 shrink-0">{{ t('round') }}:</span>
                <button
                    v-for="r in persistentRounds"
                    :key="r.round"
                    type="button"
                    @click="selectedRoundNum = r.round"
                    :class="cn(
                        'flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-xs font-medium transition-colors border shrink-0',
                        activeRound?.round === r.round
                            ? 'bg-primary text-primary-foreground border-primary shadow-xs'
                            : 'bg-muted/50 text-muted-foreground border-border hover:bg-muted hover:text-foreground',
                    )"
                >
                    <span>{{ t('roundIndex', { index: r.round }) }}:</span>
                    <span class="font-semibold">{{ r.channel_name }}</span>
                    <Badge
                        :variant="(r.status >= 200 && r.status < 300) ? 'secondary' : 'destructive'"
                        :class="cn(
                            'px-1 py-0 text-[10px]',
                            activeRound?.round === r.round && (r.status >= 200 && r.status < 300) && 'bg-primary-foreground/20 text-primary-foreground',
                        )"
                    >
                        {{ r.status || ((r.status >= 200 && r.status < 300) ? 200 : 500) }}
                    </Badge>
                    <span v-if="r.duration_ms > 0" class="opacity-70 text-[10px]">{{ formatMilliseconds(r.duration_ms) }}</span>
                </button>
            </div>

            <div class="grid grid-cols-1 md:grid-cols-2 gap-4 flex-1 min-h-0">
                <!-- 左侧: 分组与请求体 -->
                <div class="flex flex-col rounded-2xl border border-border bg-muted/30 overflow-hidden min-h-0">
                    <div class="flex h-10 shrink-0 items-center gap-2 border-b border-border bg-muted/50 pl-1 pr-3 md:pr-4">
                        <Tabs v-model="leftTab">
                            <TabsList variant="text" class="p-0">
                                <TabsTrigger value="group" class="pr-0">
                                    {{ t('group') }}
                                </TabsTrigger>
                                <span aria-hidden="true" class="mx-1 inline-flex h-full -translate-y-px items-center text-sm font-medium leading-none text-muted-foreground/50">/</span>
                                <TabsTrigger value="request" class="pl-0">
                                    {{ t('requestContent') }}
                                </TabsTrigger>
                            </TabsList>
                        </Tabs>
                        <Badge v-if="leftTab === 'request'" variant="secondary" class="ml-auto text-xs">
                            {{ (log.usage.prompt_tokens - (log.usage.cache_read_tokens ?? 0)).toLocaleString() }} {{ t('tokens') }}
                        </Badge>
                    </div>

                    <div class="flex-1 overflow-auto min-h-0">
                        <div v-if="!detailReady" class="flex h-full items-center justify-center">
                            <LuLoader2 class="size-5 animate-spin text-muted-foreground" />
                        </div>
                        <template v-else-if="leftTab === 'request'">
                            <div v-if="requestBody.isLoading.value" class="flex h-full items-center justify-center">
                                <LuLoader2 class="size-5 animate-spin text-muted-foreground" />
                            </div>
                            <div v-else-if="requestBody.error.value" class="flex h-full flex-col items-center justify-center gap-2 px-4 text-xs text-destructive">
                                <LuAlertCircle class="size-5" />
                                <span>{{ t('detailUnavailable') }}</span>
                            </div>
                            <template v-else>
                                <HttpMeta
                                    :url="activeRound?.request_url || detail.data.value?.request_url"
                                    :headers="activeRound?.request_headers || detail.data.value?.request_headers"
                                    :loading="detail.isLoading.value"
                                />
                                <JsonContent
                                    :content="activeRound?.request_body || requestBody.data.value"
                                    :fallback-text="t('noRequestContent')"
                                />
                            </template>
                        </template>
                        <div v-else-if="!activeGroup" class="flex h-full items-center justify-center px-4 text-xs text-muted-foreground">
                            {{ t('groupUnavailable') }}
                        </div>
                        <div v-else-if="!activeGroup.items?.length" class="flex h-full items-center justify-center px-4 text-xs text-muted-foreground">
                            {{ t('noGroupItems') }}
                        </div>
                        <div v-else class="divide-y divide-border">
                            <button
                                v-for="item in activeGroup.items"
                                :key="item.id ?? item.channel_model_id"
                                type="button"
                                :aria-pressed="itemCurrent(item)"
                                :disabled="item.id === undefined || activeGroup.mode === 'failover' || switchingItemId !== null || stopRound.isPending.value"
                                @click="handleSwitchItem(item)"
                                :class="cn(
                                    'flex w-full items-center gap-2.5 rounded-lg px-3 py-2.5 text-left text-xs transition-colors hover:bg-muted/50 disabled:cursor-default disabled:hover:bg-transparent',
                                    itemCurrent(item) && 'bg-muted/60 font-medium'
                                )"
                            >
                                <component
                                    :is="getItemIcon(item).Icon"
                                    aria-hidden="true"
                                    :class="getItemIcon(item).className"
                                    width="20"
                                    height="20"
                                />
                                <span class="min-w-0 flex-1">
                                    <span class="block truncate font-semibold text-foreground">
                                        {{ getItemChannelName(item) }}
                                    </span>
                                    <span class="block truncate text-[11px] text-muted-foreground">{{ item.channel_model?.name ?? '' }}</span>
                                </span>
                                <MemberStatus :group="activeGroup" :item-id="item.id" :now="now" :active="itemCurrent(item)" />
                                <LuLoader2 v-if="item.id === switchingItemId" class="size-4 shrink-0 animate-spin text-muted-foreground" />
                            </button>
                        </div>
                    </div>
                </div>

                <!-- 右侧: 响应内容或重试/错误详情 -->
                <div class="flex flex-col rounded-2xl border border-border bg-muted/30 overflow-hidden min-h-0">
                    <div class="flex h-10 shrink-0 items-center gap-2 border-b border-border bg-muted/50 px-3 md:px-4">
                        <span class="text-sm font-medium text-card-foreground">
                            {{ isWaitingForSelection ? t('waitingChannelSelection') : showRounds ? t('retryDetails') : activeRound?.error ? t('errorInfo') : requestFailed ? t('errorInfo') : t('responseContent') }}
                        </span>
                        <button
                            v-if="log.status === 'running' && log.sending && activeGroup?.mode === 'manual'"
                            type="button"
                            :disabled="stopRound.isPending.value"
                            @click="handleStopRound"
                            class="ml-auto flex items-center gap-1.5 rounded-md px-2 py-1 text-xs text-destructive transition-colors hover:bg-destructive/10 disabled:opacity-50"
                        >
                            <LuLoader2 v-if="stopRound.isPending.value" class="size-3.5 animate-spin" />
                            <LuSquare v-else class="size-3.5" />
                            {{ t('stopRound') }}
                        </button>
                        <Badge v-else-if="!requestFailed" variant="secondary" class="ml-auto text-xs">
                            {{ responseCommitted ? statusT('committed') : `${log.usage.completion_tokens.toLocaleString()} ${t('tokens')}` }}
                        </Badge>
                    </div>

                    <div class="min-h-0 flex-1 overflow-auto">
                        <div v-if="!detailReady" class="flex h-full items-center justify-center">
                            <LuLoader2 class="size-5 animate-spin text-muted-foreground" />
                        </div>
                        <div v-else-if="isWaitingForSelection" class="flex h-full items-center justify-center gap-2 text-xs text-muted-foreground">
                            <LuLoader2 class="size-4 animate-spin" />
                            {{ t('waitingChannelSelection') }}
                        </div>
                        <template v-else-if="showRounds">
                            <div v-if="rounds.length" class="divide-y divide-border">
                                <div v-for="round in rounds" :key="round.round" class="flex flex-col gap-1.5 px-3 py-2.5 text-xs">
                                    <div class="flex items-center gap-2">
                                        <span class="text-muted-foreground">{{ t('retryIndex', { index: round.round }) }}</span>
                                        <span class="font-semibold text-foreground">{{ round.channel || '-' }}</span>
                                        <LuLoader2 v-if="round.sending" class="ml-auto size-3.5 animate-spin text-muted-foreground" />
                                        <CopyButton
                                            v-else-if="round.error"
                                            :text="round.error"
                                            class="ml-auto p-1 rounded-md text-destructive/60 hover:text-destructive hover:bg-destructive/10 transition-colors"
                                            copy-icon-class-name="size-3.5"
                                            check-icon-class-name="size-3.5"
                                        />
                                    </div>
                                    <div v-if="round.error" class="text-[11px] leading-relaxed text-destructive/90 whitespace-pre-wrap break-words">
                                        {{ round.error }}
                                    </div>
                                </div>
                            </div>
                            <div v-else class="flex h-full items-center justify-center gap-2 text-xs text-muted-foreground">
                                <LuLoader2 class="size-4 animate-spin" />
                                {{ t('waitingResponse') }}
                            </div>
                        </template>
                        <div v-else-if="responseCommitted" class="flex h-full items-center justify-center gap-2 text-xs text-muted-foreground">
                            <LuLoader2 class="size-4 animate-spin" />
                            {{ t('responseStreaming') }}
                        </div>
                        <template v-else-if="activeRound">
                            <HttpMeta
                                :url="activeRound.response_url || detail.data.value?.response_url"
                                :headers="activeRound.response_headers || detail.data.value?.response_headers"
                                :loading="detail.isLoading.value"
                            />
                            <div
                            v-if="activeRound.key_attempts?.length"
                            class="mb-2 rounded-xl border border-orange-500/30 bg-orange-500/5 p-2.5 text-xs space-y-1"
                        >
                            <div class="font-medium text-orange-600 dark:text-orange-400">{{ t('keyAttempts') }}</div>
                            <div
                                v-for="(attempt, attemptIndex) in activeRound.key_attempts"
                                :key="attemptIndex"
                                class="flex items-center gap-2 text-muted-foreground"
                            >
                                <span class="font-medium text-foreground">Key #{{ attempt.key_index + 1 }}</span>
                                <Badge variant="destructive" class="px-1 py-0 text-[10px]">{{ attempt.status > 0 ? attempt.status : t('transportError') }}</Badge>
                                <span class="truncate" :title="attempt.error">{{ attempt.error }}</span>
                            </div>
                        </div>
                        <JsonContent
                            :content="activeRound.response_body || activeRound.error || errorText"
                                :fallback-text="t('noResponseContent')"
                            />
                        </template>
                        <template v-else-if="requestFailed">
                            <JsonContent :content="errorText" :fallback-text="t('noResponseContent')" />
                        </template>
                        <div v-else-if="responseBody.isLoading.value" class="flex h-full items-center justify-center">
                            <LuLoader2 class="size-5 animate-spin text-muted-foreground" />
                        </div>
                        <div v-else-if="responseBody.error.value" class="flex h-full flex-col items-center justify-center gap-2 px-4 text-xs text-destructive">
                            <LuAlertCircle class="size-5" />
                            <span>{{ t('detailUnavailable') }}</span>
                        </div>
                        <template v-else>
                            <HttpMeta
                                :url="detail.data.value?.response_url"
                                :headers="detail.data.value?.response_headers"
                                :loading="detail.isLoading.value"
                            />
                            <JsonContent :content="responseBody.data.value" :fallback-text="t('noResponseContent')" />
                        </template>
                    </div>
                </div>
            </div>
        </MorphingDialogDescription>

        <div class="flex w-full shrink-0 flex-wrap items-center gap-3 pt-4 mt-auto text-xs text-muted-foreground md:gap-4">
            <LogMetrics :log="log" :now="now" :brand-color="modelIconInfo.color" variant="footer" />
        </div>
    </MorphingDialogContent>
</template>
