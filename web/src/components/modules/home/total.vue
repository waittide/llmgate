<script setup lang="ts">
import { computed } from 'vue';
import {
    LuActivity,
    LuMessageSquare,
    LuClock,
    LuArrowDownToLine,
    LuBarChartBig,
    LuBot,
    LuArrowUpFromLine,
    LuRewind,
    LuDollarSign,
    LuFastForward,
} from 'vue-icons-plus/lu';
import { useTranslations } from '@/composables/i18n';
import { useStatsTotal } from '@/api/stats';
import AnimatedNumber from '@/components/common/AnimatedNumber.vue';

// Total 展示累计的请求, 总量, 输入和输出四组指标卡片。
const { data: stats } = useStatsTotal();
const { t } = useTranslations('home.total');

const cards = computed(() => [
    {
        title: t('requestStats'),
        headerIcon: LuActivity,
        items: [
            { label: t('requestCount'), metric: stats.value?.request_count, icon: LuMessageSquare, bgColor: 'bg-primary/10' },
            { label: t('timeConsumed'), metric: stats.value?.wait_time, icon: LuClock, bgColor: 'bg-accent/10' },
        ],
    },
    {
        title: t('totalStats'),
        headerIcon: LuBarChartBig,
        items: [
            { label: t('totalToken'), metric: stats.value?.total_token, icon: LuBot, bgColor: 'bg-chart-1/10' },
            { label: t('totalCost'), metric: stats.value?.total_cost, icon: LuDollarSign, bgColor: 'bg-chart-2/10' },
        ],
    },
    {
        title: t('inputStats'),
        headerIcon: LuArrowDownToLine,
        items: [
            { label: t('inputTokens'), metric: stats.value?.input_token, icon: LuRewind, bgColor: 'bg-chart-3/10' },
            { label: t('inputCost'), metric: stats.value?.input_cost, icon: LuDollarSign, bgColor: 'bg-chart-3/10' },
        ],
    },
    {
        title: t('outputStats'),
        headerIcon: LuArrowUpFromLine,
        items: [
            { label: t('outputTokens'), metric: stats.value?.output_token, icon: LuFastForward, bgColor: 'bg-chart-4/10' },
            { label: t('outputCost'), metric: stats.value?.output_cost, icon: LuDollarSign, bgColor: 'bg-chart-4/10' },
        ],
    },
]);
</script>

<template>
    <div class="grid grid-cols-1 @xl/home:grid-cols-2 @3xl/home:grid-cols-4 gap-4">
        <section
            v-for="card in cards"
            :key="card.title"
            class="rounded-3xl bg-card border-border border p-5 text-card-foreground flex flex-row items-center gap-4"
        >
            <div class="flex flex-col items-center justify-center gap-3 border-r border-border/50 pr-4 py-1 self-stretch">
                <component :is="card.headerIcon" class="w-4 h-4" />
                <h3 class="font-medium text-sm [writing-mode:vertical-lr]">{{ card.title }}</h3>
            </div>

            <div class="flex flex-col gap-4 flex-1 min-w-0">
                <div v-for="item in card.items" :key="item.label" class="flex items-center gap-3">
                    <div :class="['w-10 h-10 rounded-xl flex items-center justify-center shrink-0 text-primary', item.bgColor]">
                        <component :is="item.icon" class="w-5 h-5" />
                    </div>
                    <div class="flex flex-col min-w-0">
                        <span class="text-xs text-muted-foreground">{{ item.label }}</span>
                        <div class="flex items-baseline gap-1">
                            <span class="text-xl">
                                <AnimatedNumber :value="item.metric?.formatted.value" />
                            </span>
                            <!-- 计数在千位以下 unit 为空串, 空 span 仍是 flex 项, 会多出 gap-1 的间距。 -->
                            <span v-if="item.metric?.formatted.unit" class="text-sm text-muted-foreground">
                                {{ item.metric.formatted.unit }}
                            </span>
                        </div>
                    </div>
                </div>
            </div>
        </section>
    </div>
</template>
