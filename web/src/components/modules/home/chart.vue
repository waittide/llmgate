<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, useId } from 'vue';
import { useStatsDaily, useStatsHourly, type StatsMetricsFormatted } from '@/api/stats';
import AnimatedNumber from '@/components/common/AnimatedNumber.vue';
import { formatCount, formatMoney } from '@/lib/utils';
import { useTheme } from '@/composables/theme';
import { useTranslations } from '@/composables/i18n';
import { homeViewStore, type MetricKey, type ChartPeriod } from './store';
import MetricTabs from './metric-tabs.vue';

// 趋势图上的一个点。小时与每日两种粒度统一形状。
interface ChartPoint {
    label: string; // 横轴刻度, 小时粒度为 H:00, 每日粒度为 MM/DD。
    stat: Pick<StatsMetricsFormatted, 'request_count' | 'total_cost' | 'total_token'>;
}

const { data: statsDaily } = useStatsDaily();
const { data: statsHourly } = useStatsHourly();

const { t } = useTranslations('home.chart');
const { t: tMetric } = useTranslations('home.metric');
useTheme();

const rawId = useId();
const gradientId = `fillMetric-${rawId.replace(/[^a-zA-Z0-9]/g, '')}`;

// 今天取小时粒度, 其余取最近 N 天; 两种粒度的统计字段同名。
const source = computed<ChartPoint[]>(() => {
    const period = homeViewStore.chartPeriod;
    if (period === '1') {
        return (statsHourly.value ?? []).map((stat) => ({
            label: `${stat.hour}:00`,
            stat,
        }));
    }
    return [...(statsDaily.value ?? [])]
        .sort((a, b) => a.date.localeCompare(b.date))
        .slice(-Number(period))
        .map((stat) => ({
            label: `${stat.date.slice(4, 6)}/${stat.date.slice(6, 8)}`,
            stat,
        }));
});

// 图表从首个有请求的点前一个开始, 避免开头一长段零值; 汇总仍按整个周期统计。
const firstUsage = computed(() => source.value.findIndex((item) => item.stat.request_count.raw > 0));

const metricField = computed(() => {
    const type = homeViewStore.chartMetricType;
    return type === 'cost' ? 'total_cost' : type === 'count' ? 'request_count' : 'total_token';
});

const chartData = computed(() => {
    const src = source.value;
    const first = firstUsage.value;
    const startIndex = first === -1 ? Math.max(src.length - 1, 0) : Math.max(first - 1, 0);
    return src.slice(startIndex).map((item) => ({
        date: item.label,
        value: item.stat[metricField.value].raw,
    }));
});

const chartColor = computed(() => {
    if (typeof window === 'undefined') return 'oklch(0.6809 0.1201 145.0433)';
    const type = homeViewStore.chartMetricType;
    const varName = type === 'cost' ? '--chart-1' : type === 'count' ? '--chart-2' : '--chart-3';
    const computedVal = getComputedStyle(document.documentElement).getPropertyValue(varName).trim();
    if (computedVal) return computedVal;
    return type === 'cost'
        ? 'oklch(0.6809 0.1201 145.0433)'
        : type === 'count'
            ? 'oklch(0.6195 0.1100 147.8559)'
            : 'oklch(0.5605 0.1006 149.7572)';
});

const periodLabel = computed(() => {
    const period = homeViewStore.chartPeriod;
    const map: Record<ChartPeriod, string> = {
        '1': t('period.today'),
        '7': t('period.last7Days'),
        '30': t('period.last30Days'),
    };
    return map[period] || t('period.today');
});

function cyclePeriod() {
    const cur = homeViewStore.chartPeriod;
    const next: ChartPeriod = cur === '1' ? '7' : cur === '7' ? '30' : '1';
    homeViewStore.setChartPeriod(next);
}

const summary = computed(() => {
    const src = source.value;
    const totalRequests = src.reduce((sum, item) => sum + item.stat.request_count.raw, 0);
    const totalCost = src.reduce((sum, item) => sum + item.stat.total_cost.raw, 0);
    const totalTokens = src.reduce((sum, item) => sum + item.stat.total_token.raw, 0);

    return [
        { label: t('totalRequests'), metric: formatCount(totalRequests) },
        { label: t('totalCost'), metric: formatMoney(totalCost) },
        { label: t('totalTokens'), metric: formatCount(totalTokens) },
    ];
});

// --- 手绘 SVG 面积图逻辑 ---
const containerRef = ref<HTMLDivElement | null>(null);
const svgWidth = ref(600);
const svgHeight = 160;

const margin = {
    top: 16,
    right: 16,
    bottom: 26,
    left: 46,
};

let resizeObserver: ResizeObserver | null = null;

onMounted(() => {
    if (containerRef.value) {
        svgWidth.value = containerRef.value.clientWidth || 600;
        if (typeof ResizeObserver !== 'undefined') {
            resizeObserver = new ResizeObserver((entries) => {
                for (const entry of entries) {
                    if (entry.contentRect.width > 0) {
                        svgWidth.value = entry.contentRect.width;
                    }
                }
            });
            resizeObserver.observe(containerRef.value);
        }
    }
});

onUnmounted(() => {
    if (resizeObserver) {
        resizeObserver.disconnect();
        resizeObserver = null;
    }
});

const plotWidth = computed(() => Math.max(10, svgWidth.value - margin.left - margin.right));
const plotHeight = computed(() => Math.max(10, svgHeight - margin.top - margin.bottom));

const maxValue = computed(() => {
    const data = chartData.value;
    if (data.length === 0) return 1;
    const max = Math.max(...data.map((d) => d.value), 0);
    return max === 0 ? 1 : max;
});

// Y 轴刻度线（4 条横向网格线：0%, 33.3%, 66.6%, 100%）
const yTicks = computed(() => {
    const max = maxValue.value;
    const steps = 3;
    const ticks: Array<{ y: number; val: number; label: string }> = [];
    for (let i = 0; i <= steps; i++) {
        const ratio = i / steps;
        const val = max * ratio;
        const y = margin.top + plotHeight.value - ratio * plotHeight.value;
        const formatted = homeViewStore.chartMetricType === 'cost' ? formatMoney(val) : formatCount(val);
        const label = `${formatted.formatted.value}${formatted.formatted.unit}`;
        ticks.push({ y, val, label });
    }
    return ticks;
});

// 数据点在 SVG 坐标系中的投影位置
const points = computed(() => {
    const data = chartData.value;
    const n = data.length;
    const w = plotWidth.value;
    const h = plotHeight.value;
    const max = maxValue.value;

    if (n === 0) return [];
    if (n === 1) {
        const x = margin.left + w / 2;
        const y = margin.top + h - (data[0].value / max) * h;
        return [{ x, y, data: data[0], index: 0 }];
    }

    return data.map((d, index) => {
        const x = margin.left + (index / (n - 1)) * w;
        const y = margin.top + h - (d.value / max) * h;
        return { x, y, data: d, index };
    });
});

// 计算 Catmull-Rom 单调平滑三次贝塞尔曲线
const pathCommands = computed(() => {
    const pts = points.value;
    if (pts.length === 0) {
        return { line: '', area: '' };
    }
    if (pts.length === 1) {
        const p = pts[0];
        const line = `M ${p.x - 5} ${p.y} L ${p.x + 5} ${p.y}`;
        const area = `M ${p.x - 5} ${p.y} L ${p.x + 5} ${p.y} L ${p.x + 5} ${margin.top + plotHeight.value} L ${p.x - 5} ${margin.top + plotHeight.value} Z`;
        return { line, area };
    }

    let linePath = `M ${pts[0].x.toFixed(2)} ${pts[0].y.toFixed(2)}`;

    for (let i = 0; i < pts.length - 1; i++) {
        const p0 = pts[Math.max(i - 1, 0)];
        const p1 = pts[i];
        const p2 = pts[i + 1];
        const p3 = pts[Math.min(i + 2, pts.length - 1)];

        const tension = 0.2;
        const cp1x = p1.x + (p2.x - p0.x) * tension;
        const cp1y = p1.y + (p2.y - p0.y) * tension;
        const cp2x = p2.x - (p3.x - p1.x) * tension;
        const cp2y = p2.y - (p3.y - p1.y) * tension;

        linePath += ` C ${cp1x.toFixed(2)} ${cp1y.toFixed(2)}, ${cp2x.toFixed(2)} ${cp2y.toFixed(2)}, ${p2.x.toFixed(2)} ${p2.y.toFixed(2)}`;
    }

    const firstX = pts[0].x.toFixed(2);
    const lastX = pts[pts.length - 1].x.toFixed(2);
    const bottomY = (margin.top + plotHeight.value).toFixed(2);
    const areaPath = `${linePath} L ${lastX} ${bottomY} L ${firstX} ${bottomY} Z`;

    return { line: linePath, area: areaPath };
});

// X 轴刻度标签过滤（防止过多标签挤占）
const xTicks = computed(() => {
    const pts = points.value;
    if (pts.length === 0) return [];
    if (pts.length <= 6) return pts;

    const result: typeof pts = [];
    const step = (pts.length - 1) / 5;
    for (let i = 0; i < 6; i++) {
        const idx = Math.min(pts.length - 1, Math.round(i * step));
        if (!result.includes(pts[idx])) {
            result.push(pts[idx]);
        }
    }
    return result;
});

// --- Tooltip 悬浮交互 ---
const hoveredIndex = ref<number | null>(null);

const activePoint = computed(() => {
    if (hoveredIndex.value === null) return null;
    return points.value[hoveredIndex.value] ?? null;
});

const tooltipValue = computed(() => {
    if (!activePoint.value) return '';
    const val = activePoint.value.data.value;
    const formatted = homeViewStore.chartMetricType === 'cost' ? formatMoney(val) : formatCount(val);
    return `${formatted.formatted.value}${formatted.formatted.unit}`;
});

function handlePointerMove(event: MouseEvent) {
    if (!containerRef.value || points.value.length === 0) return;
    const rect = containerRef.value.getBoundingClientRect();
    const mouseX = event.clientX - rect.left;

    let closestIdx = 0;
    let minDistance = Infinity;

    for (let i = 0; i < points.value.length; i++) {
        const distance = Math.abs(points.value[i].x - mouseX);
        if (distance < minDistance) {
            minDistance = distance;
            closestIdx = i;
        }
    }

    hoveredIndex.value = closestIdx;
}

function handlePointerLeave() {
    hoveredIndex.value = null;
}
</script>

<template>
    <div class="rounded-3xl bg-card border-border border pt-2 pb-0 text-card-foreground">
        <!-- 头部标题与指标选择 -->
        <div class="px-4 pb-2 space-y-2">
            <div class="flex justify-between items-center">
                <h3 class="font-semibold text-base">{{ t('title') }}</h3>
                <MetricTabs
                    :value="homeViewStore.chartMetricType"
                    @change="homeViewStore.setChartMetricType"
                />
            </div>

            <!-- 指标统计概览与时间周期选择 -->
            <div class="flex justify-between items-start">
                <div class="flex gap-2 text-sm">
                    <template v-for="(item, index) in summary" :key="item.label">
                        <div v-if="index > 0" class="w-px bg-border self-stretch" />
                        <div>
                            <div class="text-xs text-muted-foreground">{{ item.label }}</div>
                            <div class="text-xl font-semibold">
                                <AnimatedNumber :value="item.metric.formatted.value" />
                                <span class="ml-0.5 text-sm text-muted-foreground">
                                    {{ item.metric.formatted.unit }}
                                </span>
                            </div>
                        </div>
                    </template>
                </div>

                <div
                    class="flex gap-2 text-sm cursor-pointer hover:opacity-80 transition-opacity"
                    @click="cyclePeriod"
                >
                    <div>
                        <div class="text-xs text-muted-foreground">{{ t('timePeriod') }}</div>
                        <div class="text-base font-semibold">{{ periodLabel }}</div>
                    </div>
                </div>
            </div>
        </div>

        <!-- 手绘 SVG 面积图容器 -->
        <div
            ref="containerRef"
            class="relative h-40 w-full select-none overflow-visible"
            @pointermove="handlePointerMove"
            @pointerleave="handlePointerLeave"
        >
            <svg
                :width="svgWidth"
                :height="svgHeight"
                class="overflow-visible block w-full h-full"
            >
                <defs>
                    <linearGradient :id="gradientId" x1="0" y1="0" x2="0" y2="1">
                        <stop offset="5%" :stop-color="chartColor" stop-opacity="0.85" />
                        <stop offset="95%" :stop-color="chartColor" stop-opacity="0.05" />
                    </linearGradient>
                </defs>

                <!-- 水平网格虚线 (CartesianGrid strokeDasharray="3 3") -->
                <g class="grid-lines">
                    <line
                        v-for="tick in yTicks"
                        :key="tick.y"
                        :x1="margin.left"
                        :y1="tick.y"
                        :x2="margin.left + plotWidth"
                        :y2="tick.y"
                        stroke="currentColor"
                        class="text-border/40"
                        stroke-dasharray="3 3"
                        stroke-width="1"
                    />
                </g>

                <!-- Y 轴刻度文本 -->
                <g class="y-axis">
                    <text
                        v-for="tick in yTicks"
                        :key="`y-label-${tick.y}`"
                        :x="margin.left - 8"
                        :y="tick.y + 3.5"
                        text-anchor="end"
                        class="text-[10px] fill-muted-foreground font-mono"
                    >
                        {{ tick.label }}
                    </text>
                </g>

                <!-- X 轴刻度文本 -->
                <g class="x-axis">
                    <text
                        v-for="pt in xTicks"
                        :key="`x-label-${pt.index}`"
                        :x="pt.x"
                        :y="svgHeight - 8"
                        text-anchor="middle"
                        class="text-[10px] fill-muted-foreground font-mono"
                    >
                        {{ pt.data.date }}
                    </text>
                </g>

                <!-- 渐变面积填充 -->
                <path
                    v-if="pathCommands.area"
                    :d="pathCommands.area"
                    :fill="`url(#${gradientId})`"
                />

                <!-- 平滑曲线描边 -->
                <path
                    v-if="pathCommands.line"
                    :d="pathCommands.line"
                    fill="none"
                    :stroke="chartColor"
                    stroke-width="2"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                />

                <!-- 悬浮指示线与高亮点 -->
                <g v-if="activePoint">
                    <!-- 垂直指示虚线 -->
                    <line
                        :x1="activePoint.x"
                        :y1="margin.top"
                        :x2="activePoint.x"
                        :y2="margin.top + plotHeight"
                        stroke="currentColor"
                        class="text-border"
                        stroke-dasharray="3 3"
                        stroke-width="1.5"
                    />

                    <!-- 外层光晕圈 -->
                    <circle
                        :cx="activePoint.x"
                        :cy="activePoint.y"
                        r="6"
                        :fill="chartColor"
                        fill-opacity="0.25"
                    />

                    <!-- 中心实心圆点 -->
                    <circle
                        :cx="activePoint.x"
                        :cy="activePoint.y"
                        r="3.5"
                        :fill="chartColor"
                        class="stroke-card"
                        stroke-width="2"
                    />
                </g>
            </svg>

            <!-- 悬浮 Tooltip 气泡 -->
            <div
                v-if="activePoint"
                class="absolute pointer-events-none z-20 -translate-x-1/2 -translate-y-full mb-2.5 transition-transform duration-75"
                :style="{
                    left: `${Math.max(60, Math.min(svgWidth - 60, activePoint.x))}px`,
                    top: `${Math.max(28, activePoint.y - 8)}px`,
                }"
            >
                <div class="rounded-xl border border-border/80 bg-card/95 px-3 py-1.5 shadow-lg backdrop-blur-xs text-xs">
                    <div class="text-[11px] text-muted-foreground font-medium mb-0.5">
                        {{ activePoint.data.date }}
                    </div>
                    <div class="flex items-center gap-1.5 font-semibold">
                        <span
                            class="size-2 rounded-full shrink-0"
                            :style="{ backgroundColor: chartColor }"
                        />
                        <span class="text-muted-foreground text-[11px]">{{ tMetric(homeViewStore.chartMetricType) }}:</span>
                        <span class="text-card-foreground tabular-nums">{{ tooltipValue }}</span>
                    </div>
                </div>
            </div>
        </div>
    </div>
</template>
