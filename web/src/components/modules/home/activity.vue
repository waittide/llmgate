<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount, watch, nextTick } from 'vue';
import dayjs from 'dayjs';
import { useStatsDaily, type StatsDailyFormatted } from '@/api/stats';
import { useTranslations } from '@/composables/i18n';

// 热力图上的一天。
interface ActivityDay {
    dateStr: string; // YYYYMMDD, 同时用作格子的 key 和提示的日期来源。
    formatted: StatsDailyFormatted | null; // 当日统计, 无数据为 null。
}

// Activity 以一年的日历热力图展示每日请求量, 悬浮某天时显示当日统计。
const { data: stats, maxRequestCount } = useStatsDaily();
const { t } = useTranslations('home.activity');
const scrollRef = ref<HTMLDivElement | null>(null); // 横向滚动容器, 用于贴右和判断两端。
// 悬浮提示的内容与位置; 关闭时先转为不可见再移除, 以走完淡出过渡。
const tooltip = ref<(ActivityDay & { x: number; y: number; visible: boolean }) | null>(null);
const edges = ref({ atStart: true, atEnd: true }); // 横向滚动是否已抵达两端。

// 自本周向前推 53 周的周日起逐日铺满 54 列; 今天之后的日期为 null, 只占位不渲染格子。
// 378 次 dayjs 运算加建表, 只应随统计数据变化重算。
const days = computed<(ActivityDay | null)[]>(() => {
    if (!stats.value) return [];
    const byDate = new Map(stats.value.map((stat) => [stat.date, stat]));
    const today = dayjs();
    const start = today.subtract(today.day() + 53 * 7, 'day');

    return Array.from({ length: 54 * 7 }, (_, index) => {
        const date = start.add(index, 'day');
        if (date.isAfter(today, 'day')) return null;
        const dateStr = date.format('YYYYMMDD');
        return { dateStr, formatted: byDate.get(dateStr) ?? null };
    });
});

// 记录横向滚动是否已抵达两端, 供遮罩判断淡化哪一侧。
const checkScroll = () => {
    const node = scrollRef.value;
    if (!node) return;
    edges.value = {
        atStart: node.scrollLeft <= 1,
        atEnd: Math.abs(node.scrollWidth - node.clientWidth - node.scrollLeft) <= 1,
    };
};

// 默认停在最右侧, 露出最近的日期; 窗口尺寸变化后重新贴右。
// 赋值 scrollLeft 会触发 scroll 事件, 边缘状态由 onScroll 更新, 此处无需重复计算。
const scrollToRight = () => {
    if (!scrollRef.value) return;
    scrollRef.value.scrollLeft = scrollRef.value.scrollWidth;
    checkScroll();
};

onMounted(() => {
    scrollToRight();
    window.addEventListener('resize', scrollToRight);
});

onBeforeUnmount(() => {
    window.removeEventListener('resize', scrollToRight);
});

watch(days, async () => {
    await nextTick();
    scrollToRight();
});

function getLevel(day: ActivityDay): number {
    const max = maxRequestCount.value;
    return max > 0
        ? Math.min(4, Math.ceil((day.formatted?.request_count.raw ?? 0) * 4 / max))
        : 0;
}

function handleMouseOver(event: MouseEvent) {
    const target = event.target as HTMLElement | null;
    if (!target) return;
    const indexStr = target.dataset.index;
    if (indexStr === undefined) return;
    const day = days.value[Number(indexStr)];
    if (!day) return;
    const rect = target.getBoundingClientRect();
    tooltip.value = { ...day, x: rect.left + rect.width / 2, y: rect.top, visible: true };
}

function handleMouseLeave() {
    if (tooltip.value) {
        tooltip.value.visible = false;
    }
}

// 遮罩只淡化仍可继续滚动的那一侧, 两端都到头则不淡化。
const maskImage = computed(() => {
    if (edges.value.atStart && edges.value.atEnd) return 'none';
    return `linear-gradient(to right, ${[
        edges.value.atStart ? 'black 0' : 'transparent, rgba(0,0,0,0) 10px, black 40px',
        edges.value.atEnd ? 'black 100%' : 'black calc(100% - 40px), rgba(0,0,0,0) calc(100% - 10px), transparent',
    ].join(', ')})`;
});

const tooltipTransform = computed(() => {
    if (!tooltip.value) return '';
    const winWidth = typeof window !== 'undefined' ? window.innerWidth : 1000;
    const winHeight = typeof window !== 'undefined' ? window.innerHeight : 800;
    const tx = tooltip.value.x < 200 ? '10%' : tooltip.value.x > winWidth - 200 ? '-110%' : '-50%';
    const ty = tooltip.value.y < winHeight / 2 ? '15%' : '-105%';
    return `translate(${tx}, ${ty})`;
});
</script>

<template>
    <div class="rounded-3xl bg-card border-border border text-card-foreground">
        <div
            ref="scrollRef"
            @scroll="checkScroll"
            @mouseleave="handleMouseLeave"
            class="overflow-x-auto p-4"
            :style="{ maskImage, WebkitMaskImage: maskImage }"
        >
            <div class="ml-auto w-fit">
                <div
                    class="grid gap-1"
                    style="grid-template-columns: repeat(54, 0.875rem); grid-template-rows: repeat(7, 0.875rem); grid-auto-flow: column;"
                    @mouseover="handleMouseOver"
                >
                    <template v-for="(day, index) in days" :key="day ? day.dateStr : `future-${index}`">
                        <div v-if="!day" />
                        <div
                            v-else
                            :data-index="index"
                            class="rounded-sm transition-all cursor-pointer hover:scale-150"
                            :style="{
                                backgroundColor: getLevel(day) === 0 ? 'var(--muted)' : 'var(--primary)',
                                opacity: getLevel(day) === 0 ? 1 : getLevel(day) / 4,
                            }"
                        />
                    </template>
                </div>
            </div>
        </div>

        <Teleport to="body">
            <div
                v-if="tooltip"
                :class="[
                    'fixed z-50 w-fit min-w-max text-sm bg-background text-foreground border rounded-3xl p-3 transition-opacity duration-500 pointer-events-none',
                    tooltip.visible ? 'opacity-100' : 'opacity-0',
                ]"
                :style="{
                    left: `${tooltip.x}px`,
                    top: `${tooltip.y}px`,
                    transform: tooltipTransform,
                }"
            >
                <div class="space-y-2">
                    <p class="font-semibold text-foreground">
                        {{ `${tooltip.dateStr.slice(0, 4)}-${tooltip.dateStr.slice(4, 6)}-${tooltip.dateStr.slice(6, 8)}` }}
                    </p>
                    <div v-if="tooltip.formatted" class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 items-center text-muted-foreground">
                        <template
                            v-for="[labelKey, metric] in [
                                ['requestCount', tooltip.formatted.request_count],
                                ['waitTime', tooltip.formatted.wait_time],
                                ['totalToken', tooltip.formatted.total_token],
                                ['totalCost', tooltip.formatted.total_cost],
                            ] as const"
                            :key="labelKey"
                        >
                            <span class="wrap-break-word">{{ t(labelKey) }}</span>
                            <span class="text-foreground font-medium text-right">
                                {{ metric.formatted.value }}{{ metric.formatted.unit }}
                            </span>
                        </template>
                    </div>
                    <p v-else class="text-muted-foreground">{{ t('noData') }}</p>
                </div>
            </div>
        </Teleport>
    </div>
</template>
