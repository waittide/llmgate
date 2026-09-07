<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watchEffect } from 'vue';
import { LuAlertCircle, LuArrowRight, LuCheckCircle2, LuLoader2, LuSquare } from 'vue-icons-plus/lu';
import {
    type RelayLogOverview,
    type RoundLog,
    useLogDetail,
    useLogRequestBody,
    useLogResponseBody,
    useStopRound,
} from '@/api/log';
import { useGroupList } from '@/api/group';
import { useChannelList } from '@/api/channel';
import { Badge } from '@/components/ui/badge';
import { useTranslations } from '@/composables/i18n';
import { toast } from '@/composables/toast';
import { cn } from '@/lib/utils';
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

const rounds = ref<ObservedRound[]>([]);
const observedRoundKey = ref('');
const detailReady = ref(false);

const requestBody = useLogRequestBody(
    computed(() => props.log.id),
    computed(() => props.log.started_at),
    computed(() => detailReady.value),
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

const stopRound = useStopRound();

const { data: groupsData } = useGroupList(
    computed(() => detailReady.value),
    computed(() => detailReady.value),
);
const groups = computed(() => groupsData.value ?? []);

const { data: channelsData } = useChannelList(computed(() => detailReady.value));
const channels = computed(() => channelsData.value ?? []);

const activeGroup = computed(() => groups.value.find((group) => group.name.toLowerCase() === props.log.model?.toLowerCase()));

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
const roundSuccess = (r: RoundLog) => r.status >= 200 && r.status < 300;

type ChipState = 'success' | 'failed' | 'unused';
interface KeyChip {
    keyIndex: number;          // 渠道内 key 序号(0 基)
    state: ChipState;          // success=该 key 轮次成功; failed=轮换失败; unused=未使用
    round: RoundLog | null;    // 使用该 key 的轮次; null = 未使用
}
interface ChannelChip {
    name: string;
    state: ChipState;          // 渠道任一轮成功=success; 有尝试但全失败=failed; 未尝试=unused
    keys: KeyChip[];
    selectRound: number | null; // 点击色块选中的轮次(成功轮优先, 否则首次尝试)
}

// 渠道内 key 总数: 与后端 key_pool 同规则(换行/逗号分隔, 去空白)
function keyCountOf(key: string): number {
    return key.replace(',', '\n').split('\n').map((k) => k.trim()).filter((k) => k.length > 0).length || 1;
}


// channelSequence 按分组中渠道的真实顺序(priority)排列全部渠道;
// 密钥行只展示当前选中分组的 keys。使用过的标记结果, 未使用的置灰。
const channelSequence = computed<ChannelChip[]>(() => {
    const group = activeGroup.value;
    if (!group) return [];

    const roundsByChannel = new Map<string, RoundLog[]>();
    for (const r of persistentRounds.value) {
        const list = roundsByChannel.get(r.channel_name) ?? [];
        list.push(r);
        roundsByChannel.set(r.channel_name, list);
    }

    const keyCountByChannel = new Map<number, number>();
    const nameByChannelId = new Map<number, string>();
    for (const item of [...(group.items ?? [])].sort((a, b) => a.priority - b.priority)) {
        const channelId = item.channel_model?.channel_id;
        if (channelId === undefined) continue;
        const channel = channels.value.find((c) => c.raw.id === channelId)?.raw;
        if (!channel) continue;
        keyCountByChannel.set(channelId, keyCountOf(channel.key));
        nameByChannelId.set(channelId, channel.name);
    }

    const seq: ChannelChip[] = [];
    for (const [channelId, name] of nameByChannelId) {
        const rounds = roundsByChannel.get(name);
        const keyCount = keyCountByChannel.get(channelId) ?? 0;
        const keys: KeyChip[] = [];
        for (let idx = 0; idx < keyCount; idx++) {
            const round = rounds?.find((r) => r.key_index === idx) ?? null;
            keys.push({
                keyIndex: idx,
                state: round ? (roundSuccess(round) ? 'success' : 'failed') : 'unused',
                round,
            });
        }
        const used = Boolean(rounds?.length);
        const success = (rounds ?? []).some((r) => roundSuccess(r));
        const successRound = (rounds ?? []).find((r) => roundSuccess(r));
        seq.push({
            name,
            state: success ? 'success' : used ? 'failed' : 'unused',
            keys,
            selectRound: success ? (successRound?.round ?? null) : (rounds?.[0]?.round ?? null),
        });
    }
    return seq;
});

// 当前选中的分组: 未手动选择时跟随最后一轮(默认为成功/最终轮的分组)
const selectedChannelName = ref<string | null>(null);
const selectedChannelKeys = computed<KeyChip[]>(() => {
    const name = selectedChannelName.value ?? activeRound.value?.channel_name ?? null;
    return channelSequence.value.find((c) => c.name === name)?.keys ?? [];
});

function selectChannel(c: ChannelChip) {
    selectedChannelName.value = c.name;
    if (c.selectRound !== null) selectedRoundNum.value = c.selectRound;
}

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
                class="text-[11px] ml-auto mr-6"
            >
                {{ t('roundIndex', { index: activeRound?.round ?? log.round }) }} / {{ persistentRounds.length }}
            </Badge>
        </MorphingDialogTitle>

        <MorphingDialogDescription class="flex-1 min-h-0 flex flex-col gap-2">
            <div class="flex items-center gap-1.5 overflow-x-auto pb-1 shrink-0 text-xs">
                <span class="text-muted-foreground shrink-0 font-medium">{{ t('group') }}:</span>
                <button
                    v-for="c in channelSequence"
                    :key="`c-${c.name}`"
                    type="button"
                    @click="selectChannel(c)"
                    :class="cn(
                        'shrink-0 px-2 py-0.5 rounded-md border text-xs font-medium transition-colors inline-flex items-center gap-1',
                        c.state === 'success' ? 'border-emerald-500/40 bg-emerald-500/5 text-emerald-600 dark:text-emerald-400' : '',
                        c.state === 'failed' ? 'border-destructive/40 bg-destructive/5 text-destructive' : '',
                        c.state === 'unused' ? 'border-border/60 bg-muted/30 text-muted-foreground/40 cursor-default' : '',
                        selectedChannelName === c.name && 'border-primary bg-primary/10 text-primary',
                    )"
                >
                    <LuCheckCircle2 v-if="c.state === 'success'" class="size-3.5" />
                    {{ c.name }}
                </button>
            </div>
            <div class="flex items-center gap-1.5 overflow-x-auto pb-1 shrink-0 text-xs">
                <span class="text-muted-foreground shrink-0 font-medium">{{ t('key') }}:</span>
                <button
                    v-for="k in selectedChannelKeys"
                    :key="`k-${k.keyIndex}`"
                    type="button"
                    :disabled="k.state === 'unused'"
                    @click="k.round && (selectedRoundNum = k.round.round)"
                    :class="cn(
                        'shrink-0 px-2 py-0.5 rounded-md border text-xs font-medium transition-colors inline-flex items-center gap-1',
                        k.state === 'success' ? 'border-emerald-500/40 bg-emerald-500/5 text-emerald-600 dark:text-emerald-400' : '',
                        k.state === 'failed' ? 'border-destructive/40 bg-destructive/5 text-destructive' : '',
                        k.state === 'unused' ? 'border-border/60 bg-muted/30 text-muted-foreground/40 cursor-default' : '',
                        activeRound?.round === k.round?.round && 'border-primary bg-primary/10 text-primary',
                    )"
                >
                    <LuCheckCircle2 v-if="k.state === 'success'" class="size-3" />
                    {{ t('key') }}{{ k.keyIndex + 1 }}
                </button>
            </div>

            <div class="grid grid-cols-1 md:grid-cols-2 gap-4 flex-1 min-h-0">
                <!-- 左侧: 选中轮次的请求信息 -->
                <div class="flex flex-col rounded-2xl border border-border bg-muted/30 overflow-hidden min-h-0">
                    <div class="flex h-10 shrink-0 items-center gap-2 border-b border-border bg-muted/50 px-3 md:px-4">
                        <span class="text-sm font-medium text-card-foreground">{{ t('requestContent') }}</span>
                        <Badge variant="secondary" class="ml-auto text-xs">
                            {{ (log.usage.prompt_tokens - (log.usage.cache_read_tokens ?? 0)).toLocaleString() }} {{ t('tokens') }}
                        </Badge>
                    </div>

                    <div class="flex-1 overflow-auto min-h-0">
                        <div v-if="!detailReady" class="flex h-full items-center justify-center">
                            <LuLoader2 class="size-5 animate-spin text-muted-foreground" />
                        </div>
                        <div v-else-if="requestBody.isLoading.value" class="flex h-full items-center justify-center">
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
                    </div>
                </div>

                <!-- 右侧: 选中轮次的响应内容或重试/错误详情 -->
                <div class="flex flex-col rounded-2xl border border-border bg-muted/30 overflow-hidden min-h-0">
                    <div class="flex h-10 shrink-0 items-center gap-2 border-b border-border bg-muted/50 px-3 md:px-4">
                        <span class="text-sm font-medium text-card-foreground">
                            {{ showRounds ? t('retryDetails') : activeRound?.error ? t('errorInfo') : requestFailed ? t('errorInfo') : t('responseContent') }}
                        </span>
                        <button
                            v-if="log.status === 'running' && log.sending"
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
                            <JsonContent
                                :content="activeRound.response_body || activeRound.error || errorText"
                                :fallback-text="t('noResponseContent')"
                                :collapsed="roundSuccess(activeRound)"
                            />
                        </template>
                        <template v-else-if="requestFailed">
                            <JsonContent :content="errorText" :fallback-text="t('noResponseContent')" />
                        </template>
                        <div v-else-if="responseBody.isLoading.value" class="flex h-full items-center justify-center">
                            <LuLoader2 class="size-5 animate-spin text-muted-foreground" />
                        </div>
                    </div>
                </div>
            </div>

            <!-- 底部指标栏: 占满整行 -->
            <div class="grid grid-cols-7 gap-x-4 shrink-0 border-t border-border pt-2 text-xs min-w-0">
                    <LogMetrics :log="log" :now="now" :brand-color="modelIconInfo.color" variant="footer" />
                </div>
            </MorphingDialogDescription>
    </MorphingDialogContent>
</template>
