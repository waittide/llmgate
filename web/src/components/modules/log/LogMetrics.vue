<script setup lang="ts">
import { computed, type CSSProperties } from 'vue';
import {
    LuClock,
    LuCpu,
    LuDollarSign,
    LuArrowDownToLine,
    LuDatabase,
    LuArrowUpFromLine,
} from 'vue-icons-plus/lu';
import { cn } from '@/lib/utils';
import type { RelayLogOverview } from '@/api/log';

export interface LogMetricsProps {
    log: RelayLogOverview;
    now: number;
    brandColor: string;
    variant: 'card' | 'footer';
}

const props = defineProps<LogMetricsProps>();

function formatTime(value: string) {
    const date = new Date(value);
    if (Number.isNaN(date.getTime()) || date.getUTCFullYear() === 1) return '--';
    const now = new Date();
    const isSameDay =
        date.getFullYear() === now.getFullYear() &&
        date.getMonth() === now.getMonth() &&
        date.getDate() === now.getDate();
    const time = date.toLocaleTimeString(undefined, {
        hour: '2-digit',
        minute: '2-digit',
        second: '2-digit',
        hour12: false,
    });
    if (isSameDay) return time;
    const month = String(date.getMonth() + 1).padStart(2, '0');
    const day = String(date.getDate()).padStart(2, '0');
    return `${month}-${day} ${time}`;
}

function formatMilliseconds(value: number) {
    const milliseconds = Math.max(0, value);
    if (milliseconds < 1000) return `${Math.round(milliseconds)}ms`;
    return `${(milliseconds / 1000).toFixed(2)}s`;
}

const metrics = computed(() => {
    const cachedTokens = props.log.usage.cache_read_tokens ?? 0;
    const duration = props.log.status === 'running' || props.log.status === 'committed'
        ? formatMilliseconds(props.now - new Date(props.log.started_at).getTime())
        : formatMilliseconds(props.log.duration);

    return [
        { key: 'time', Icon: LuClock, iconClassName: 'size-3.5 shrink-0', iconStyle: { color: props.brandColor } as CSSProperties, value: formatTime(props.log.started_at), valueClassName: 'tabular-nums', cellClassName: 'col-span-4 whitespace-nowrap md:col-span-1' },
        { key: 'duration', Icon: LuCpu, iconClassName: 'size-3.5 shrink-0 text-blue-500', value: duration, cellClassName: 'col-span-4 md:col-span-1' },
        { key: 'cost', Icon: LuDollarSign, iconClassName: 'size-3.5 shrink-0 text-emerald-500', value: props.log.cost.toFixed(6), valueClassName: 'font-medium text-emerald-600 dark:text-emerald-400', cellClassName: 'col-span-4 md:col-span-1' },
        { key: 'prompt', Icon: LuArrowDownToLine, iconClassName: 'size-3.5 shrink-0 text-green-500', value: (props.log.usage.prompt_tokens - cachedTokens).toLocaleString(), cellClassName: 'col-span-3 md:col-span-1' },
        { key: 'cached', Icon: LuDatabase, iconClassName: 'size-3.5 shrink-0 text-cyan-500', value: cachedTokens.toLocaleString(), cellClassName: 'col-span-3 md:col-span-1' },
        { key: 'completion', Icon: LuArrowUpFromLine, iconClassName: 'size-3.5 shrink-0 text-purple-500', value: props.log.usage.completion_tokens.toLocaleString(), cellClassName: 'col-span-3 md:col-span-1' },
        { key: 'cacheWrite', Icon: LuDatabase, iconClassName: 'size-3.5 shrink-0 text-orange-500', value: (props.log.usage.cache_write_tokens ?? 0).toLocaleString(), cellClassName: 'col-span-3 md:col-span-1' },
    ];
});
</script>

<template>
    <template v-for="metric in metrics" :key="metric.key">
        <div :class="cn('flex items-center gap-1.5 min-w-0', variant === 'card' && metric.cellClassName)">
            <component :is="metric.Icon" :class="metric.iconClassName" :style="metric.iconStyle" />
            <span :class="cn('truncate', metric.valueClassName)" :title="metric.value">{{ metric.value }}</span>
        </div>
    </template>
</template>
