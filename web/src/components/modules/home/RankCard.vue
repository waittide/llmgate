<script setup lang="ts">
import { computed } from 'vue';
import { LuTrendingUp } from 'vue-icons-plus/lu';
import { useTranslations } from '@/composables/i18n';
import type { StatsMetricsFormatted } from '@/api/stats';
import type { MetricKey } from './store';
import MetricTabs from './metric-tabs.vue';

// 榜单只展示这几项, 渠道可直接复用 api/channel 已算好的 formatted。
export type RankMetrics = Pick<
    StatsMetricsFormatted,
    'total_cost' | 'total_token' | 'request_count' | 'request_success' | 'request_failed'
>;

// 榜单中的一个条目, 渠道和模型共用。
export interface RankItem {
    id: string;
    name: string; // 渠道榜为渠道名, 模型榜为模型名。
    channelName?: string; // 仅模型榜有值; 有值则 name 是模型名, 模糊渠道名时只糊此项。
    formatted: RankMetrics;
}

export interface RankCardProps {
    title: string;
    items: RankItem[];
    sortMode: MetricKey;
    hideChannelName?: boolean;
}

const props = defineProps<RankCardProps>();
const emit = defineEmits<{
    (e: 'update:sortMode', value: MetricKey): void;
}>();

const { t } = useTranslations('home.rank');
const sortField = computed(() =>
    props.sortMode === 'cost' ? 'total_cost' : props.sortMode === 'count' ? 'request_count' : 'total_token'
);
const ranked = computed(() =>
    [...props.items].sort((a, b) => b.formatted[sortField.value].raw - a.formatted[sortField.value].raw)
);
</script>

<template>
    <div class="rounded-3xl bg-card text-card-foreground border-border border pt-2 px-4">
        <div class="flex items-center justify-between">
            <h3 class="font-semibold text-base">{{ title }}</h3>
            <MetricTabs :value="sortMode" @change="(val) => emit('update:sortMode', val)" />
        </div>

        <div v-if="ranked.length === 0" class="flex flex-col items-center justify-center py-8 text-muted-foreground">
            <LuTrendingUp class="w-12 h-12 mb-3 opacity-30" />
            <p class="text-sm">{{ t('noData') }}</p>
        </div>

        <div v-else class="space-y-3 max-h-[300px] overflow-y-auto">
            <div
                v-for="(item, index) in ranked"
                :key="item.id"
                class="grid grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-3 py-3"
            >
                <div class="flex items-center justify-center font-bold text-lg">{{ index + 1 }}</div>

                <div class="min-w-0">
                    <p :class="['font-medium text-sm truncate', hideChannelName && !item.channelName ? 'select-none blur-[3px]' : '']">
                        {{ item.name }}
                    </p>
                    <p v-if="item.channelName" :class="['mt-1 truncate text-xs text-muted-foreground', hideChannelName ? 'select-none blur-[3px]' : '']">
                        {{ item.channelName }}
                    </p>
                    <div v-if="sortMode === 'count'" class="flex items-center gap-1 text-xs text-muted-foreground mt-1">
                        <span>{{ t('successRate') }}:</span>
                        <span>{{ (item.formatted.request_success.raw + item.formatted.request_failed.raw > 0 ? (item.formatted.request_success.raw / (item.formatted.request_success.raw + item.formatted.request_failed.raw)) * 100 : 0).toFixed(1) }}%</span>
                    </div>
                </div>

                <div class="flex items-center gap-1 text-right">
                    <div v-if="sortMode === 'count'" class="flex items-center gap-1 text-sm font-medium tabular-nums">
                        <span class="text-accent">
                            {{ item.formatted.request_success.formatted.value }}
                            <span class="text-xs text-muted-foreground">
                                {{ item.formatted.request_success.formatted.unit }}
                            </span>
                        </span>
                        <span class="text-muted-foreground/40 font-light">/</span>
                        <span class="text-destructive">
                            {{ item.formatted.request_failed.formatted.value }}
                            <span class="text-xs text-muted-foreground">
                                {{ item.formatted.request_failed.formatted.unit }}
                            </span>
                        </span>
                    </div>
                    <span v-else class="font-semibold text-base">
                        {{ item.formatted[sortField].formatted.value }}
                        <span class="text-xs text-muted-foreground">
                            {{ item.formatted[sortField].formatted.unit }}
                        </span>
                    </span>
                </div>
            </div>
        </div>
    </div>
</template>
