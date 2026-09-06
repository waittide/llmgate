<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watchEffect } from 'vue';
import { LuAlertCircle, LuArrowRight, LuLoader2, LuSquare } from 'vue-icons-plus/lu';
import {
    type RelayLogOverview,
    type RoundLog,
    useLogDetail,
    useLogRequestBody,
    useLogResponseBody,
    useStopRound,
} from '@/api/log';
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
                class="text-[11px] ml-auto"
            >
                {{ t('roundIndex', { index: activeRound?.round ?? log.round }) }} / {{ persistentRounds.length }}
            </Badge>
        </MorphingDialogTitle>

        <MorphingDialogDescription class="flex-1 min-h-0 flex flex-col gap-2">
            <!-- 渠道使用顺序色块: 点击切换查看对应轮次; 成功绿标, 未产生最终结果的置灰 -->
            <div class="flex items-center gap-1.5 overflow-x-auto pb-1 shrink-0 text-xs">
                <span class="text-muted-foreground shrink-0 font-medium">[{{ t('group') }}]:</span>
                <button
                    v-for="(r, i) in persistentRounds"
                    :key="`c${i}`"
                    type="button"
                    @click="selectedRoundNum = r.round"
                    :class="cn(
                        'shrink-0 px-2 py-0.5 rounded-md border text-xs font-medium transition-colors',
                        activeRound?.round === r.round
                            ? 'border-primary bg-primary/10 text-primary shadow-xs'
                            : roundSuccess(r)
                                ? 'border-emerald-500/40 bg-emerald-500/5 text-emerald-600 dark:text-emerald-400'
                                : 'border-border bg-muted/50 text-muted-foreground/60 hover:bg-muted hover:text-foreground',
                    )"
                >
                    {{ r.channel_name }}{{ roundSuccess(r) ? ' ✓' : '' }}
                </button>
            </div>
            <!-- key 使用顺序色块: 轮换中间失败的删除线置灰, 最终 key 按轮次结果标记 -->
            <div class="flex items-center gap-1.5 overflow-x-auto pb-1 shrink-0 text-xs">
                <span class="text-muted-foreground shrink-0 font-medium">[{{ t('key') }}]:</span>
                <template v-for="(r, ri) in persistentRounds" :key="`kr${ri}`">
                    <button
                        v-for="(a, ai) in r.key_attempts ?? []"
                        :key="`f${ri}-${ai}`"
                        type="button"
                        @click="selectedRoundNum = r.round"
                        :class="cn(
                            'shrink-0 px-2 py-0.5 rounded-md border text-xs line-through transition-colors',
                            activeRound?.round === r.round
                                ? 'border-destructive/50 bg-destructive/5 text-destructive/80'
                                : 'border-border bg-muted/50 text-muted-foreground/50 hover:bg-muted',
                        )"
                    >
                        {{ t('key') }}{{ a.key_index + 1 }}
                    </button>
                    <button
                        type="button"
                        @click="selectedRoundNum = r.round"
                        :class="cn(
                            'shrink-0 px-2 py-0.5 rounded-md border text-xs font-medium transition-colors',
                            activeRound?.round === r.round
                                ? (roundSuccess(r)
                                    ? 'border-emerald-500/40 bg-emerald-500/5 text-emerald-600 dark:text-emerald-400'
                                    : 'border-primary/50 bg-primary/10 text-primary')
                                : roundSuccess(r)
                                    ? 'border-emerald-500/40 text-emerald-600 dark:text-emerald-400'
                                    : 'border-border bg-muted/50 text-muted-foreground/60 hover:bg-muted',
                        )"
                    >
                        {{ t('key') }}{{ (r.key_index ?? 0) + 1 }}{{ roundSuccess(r) ? ' ✓' : '' }}
                    </button>
                    <span v-if="ri < persistentRounds.length - 1" class="text-muted-foreground/40 shrink-0">→</span>
                </template>
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
                                    <span class="font-medium text-foreground">{{ t('key') }}{{ attempt.key_index + 1 }}</span>
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
                    </div>
                </div>
            </div>

            <LogMetrics :log="log" :now="now" :brand-color="modelIconInfo.color" variant="footer" />
        </MorphingDialogDescription>
    </MorphingDialogContent>
</template>
