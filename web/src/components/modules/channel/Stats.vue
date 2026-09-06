<script setup lang="ts">
import { ref, computed, watch, onBeforeUnmount } from 'vue';
import {
    LuActivity,
    LuCheckCircle2,
    LuClock,
    LuCopy,
    LuDollarSign,
    LuDownload,
    LuEye,
    LuEyeOff,
    LuFileText,
    LuMessageSquare,
    LuShare2,
    LuX,
} from 'vue-icons-plus/lu';
import { snapdom } from '@zumer/snapdom';
import { toast } from '@/composables/toast';
import { useTranslations } from '@/composables/i18n';
import type { Channel } from '@/api/channel';
import type { StatsMetricsFormatted } from '@/api/stats';
import { formatCount, formatMoney } from '@/lib/utils';

const props = defineProps<{
    channel: Channel;
    stats: StatsMetricsFormatted;
}>();

// 模型统计的排序维度
type ModelSortKey = 'cost' | 'count' | 'tokens';

// 成功率百分比, 无请求时按 0 处理
const successRate = (success: number, failed: number) => {
    const total = success + failed;
    return total > 0 ? (success / total) * 100 : 0;
};

const { t } = useTranslations('channel.stats');
const { t: tCommon } = useTranslations('common');
const modelSort = ref<ModelSortKey>('cost');
// 隐藏渠道名称, 分享图也跟随此状态
const isNameHidden = ref(false);
const isSharing = ref(false);
// 分享图预览, url 用于展示, blob 用于复制和下载
const preview = ref<{ url: string; blob: Blob } | null>(null);
// 分享截图的源节点
const contentRef = ref<HTMLDivElement | null>(null);

// 预览关闭或组件卸载时释放临时对象 URL
watch(preview, (newVal, oldVal) => {
    if (oldVal) URL.revokeObjectURL(oldVal.url);
});

onBeforeUnmount(() => {
    if (preview.value) URL.revokeObjectURL(preview.value.url);
});

// 模型级统计: 现场格式化原始计数, 按当前维度排序并计算占比
const modelStats = computed(() => {
    const items = props.channel.models.map((model) => {
        const count = model.request_success + model.request_failed;
        const tokens = model.input_token + model.output_token;
        const cost = model.input_cost + model.output_cost;
        return {
            id: model.id,
            name: model.name,
            weight: modelSort.value === 'cost' ? cost : modelSort.value === 'tokens' ? tokens : count,
            rate: successRate(model.request_success, model.request_failed),
            count: formatCount(count),
            tokens: formatCount(tokens),
            cost: formatMoney(cost),
        };
    });

    const total = items.reduce((sum, item) => sum + item.weight, 0);
    return items
        .sort((a, b) => b.weight - a.weight || a.name.localeCompare(b.name))
        .map((item) => ({ ...item, share: total > 0 ? (item.weight / total) * 100 : 0 }));
});

const sortOptions: { key: ModelSortKey; label: string }[] = [
    { key: 'cost', label: t('sortByCost') },
    { key: 'count', label: t('sortByCount') },
    { key: 'tokens', label: t('sortByTokens') },
];

// 把统计区克隆进屏外的固定宽度卡片再截图, 容器查询使分享图不受当前屏幕宽度影响
const handleShare = async () => {
    if (!contentRef.value) return;

    const stage = document.createElement('div');
    stage.className = 'rounded-3xl bg-card px-4 py-2 text-card-foreground';
    // 宽度与桌面端弹窗卡片一致, 定位到屏外避免闪动
    Object.assign(stage.style, { position: 'fixed', top: '0', left: '-10000px', width: '768px' });
    stage.appendChild(contentRef.value.cloneNode(true));
    document.body.appendChild(stage);

    isSharing.value = true;
    try {
        const blob = await snapdom.toBlob(stage, {
            type: 'png',
            scale: 2,
            embedFonts: true,
            exclude: ['[data-share-exclude]'],
            excludeMode: 'remove',
        });
        preview.value = { url: URL.createObjectURL(blob), blob };
    } catch (error) {
        toast.error(error instanceof Error ? error.message : String(error));
    } finally {
        stage.remove();
        isSharing.value = false;
    }
};

const handleCopyImage = async () => {
    if (!preview.value) return;
    try {
        await navigator.clipboard.write([new ClipboardItem({ 'image/png': preview.value.blob })]);
        toast.success(tCommon('copy.success'));
    } catch {
        toast.error(tCommon('copy.failed'));
    }
};

const handleDownloadImage = () => {
    if (!preview.value) return;
    const link = document.createElement('a');
    link.href = preview.value.url;
    link.download = `channel-${props.channel.id}.png`;
    link.click();
};

const iconButtonClass = 'flex size-6 items-center justify-center rounded-md text-muted-foreground/60 transition-colors hover:text-foreground disabled:opacity-50';
const previewActionClass = 'flex size-10 items-center justify-center rounded-xl border border-border bg-card text-muted-foreground transition-colors hover:text-foreground active:scale-95';
</script>

<template>
    <div ref="contentRef" class="@container/stats cursor-default">
        <div class="grid gap-4 pb-2 @2xl/stats:grid-cols-[minmax(0,260px)_minmax(0,1fr)]">
            <!-- 左列: 渠道汇总 -->
            <section class="flex flex-col gap-2">
                <div class="flex h-7 items-center gap-1">
                    <h3 :class="['truncate text-xs font-semibold tracking-wider text-muted-foreground', isNameHidden ? 'select-none blur-[3px]' : '']">
                        {{ channel.name }}
                    </h3>
                    <div data-share-exclude class="flex shrink-0 items-center">
                        <button
                            type="button"
                            @click="isNameHidden = !isNameHidden"
                            :aria-pressed="isNameHidden"
                            :class="iconButtonClass"
                        >
                            <LuEyeOff v-if="isNameHidden" class="size-3.5" />
                            <LuEye v-else class="size-3.5" />
                        </button>
                        <button type="button" @click="handleShare" :disabled="isSharing" :class="iconButtonClass">
                            <LuShare2 class="size-3.5" />
                        </button>
                    </div>
                </div>
                <dl class="grid grid-cols-2 gap-2 @md/stats:grid-cols-4 @2xl/stats:grid-cols-1 @2xl/stats:flex-1 @2xl/stats:auto-rows-fr">
                    <!-- 总请求 -->
                    <div class="flex flex-col rounded-2xl border bg-card p-3">
                        <dt class="flex items-center gap-1.5 text-xs text-muted-foreground">
                            <LuActivity class="size-3.5 text-chart-1" />
                            <span class="truncate">{{ t('totalRequests') }}</span>
                        </dt>
                        <dd class="mt-auto pt-2 text-right">
                            <span class="block text-lg font-bold tabular-nums text-card-foreground">
                                {{ stats.request_count.formatted.value }}
                                <span v-if="stats.request_count.formatted.unit" class="ml-0.5 text-xs font-normal text-muted-foreground">{{ stats.request_count.formatted.unit }}</span>
                            </span>
                            <span class="mt-0.5 flex flex-wrap items-center justify-end gap-x-2 text-[11px] tabular-nums text-muted-foreground">
                                <span class="text-accent">{{ stats.request_success.formatted.value }}</span>
                                <span class="text-muted-foreground/40">/</span>
                                <span class="text-destructive">{{ stats.request_failed.formatted.value }}</span>
                                <span class="text-muted-foreground/40">·</span>
                                <span>{{ successRate(stats.request_success.raw, stats.request_failed.raw).toFixed(1) }}%</span>
                            </span>
                        </dd>
                    </div>

                    <!-- 总词元 -->
                    <div class="flex flex-col rounded-2xl border bg-card p-3">
                        <dt class="flex items-center gap-1.5 text-xs text-muted-foreground">
                            <LuFileText class="size-3.5 text-chart-3" />
                            <span class="truncate">{{ t('totalToken') }}</span>
                        </dt>
                        <dd class="mt-auto pt-2 text-right">
                            <span class="block text-lg font-bold tabular-nums text-card-foreground">
                                {{ stats.total_token.formatted.value }}
                                <span v-if="stats.total_token.formatted.unit" class="ml-0.5 text-xs font-normal text-muted-foreground">{{ stats.total_token.formatted.unit }}</span>
                            </span>
                            <span class="mt-0.5 flex flex-wrap items-center justify-end gap-x-2 text-[11px] tabular-nums text-muted-foreground">
                                <span>↓ {{ stats.input_token.formatted.value }}{{ stats.input_token.formatted.unit }}</span>
                                <span>↑ {{ stats.output_token.formatted.value }}{{ stats.output_token.formatted.unit }}</span>
                            </span>
                        </dd>
                    </div>

                    <!-- 总成本 -->
                    <div class="flex flex-col rounded-2xl border bg-card p-3">
                        <dt class="flex items-center gap-1.5 text-xs text-muted-foreground">
                            <LuDollarSign class="size-3.5 text-chart-5" />
                            <span class="truncate">{{ t('totalCost') }}</span>
                        </dt>
                        <dd class="mt-auto pt-2 text-right">
                            <span class="block text-lg font-bold tabular-nums text-card-foreground">
                                {{ stats.total_cost.formatted.value }}
                                <span v-if="stats.total_cost.formatted.unit" class="ml-0.5 text-xs font-normal text-muted-foreground">{{ stats.total_cost.formatted.unit }}</span>
                            </span>
                            <span class="mt-0.5 flex flex-wrap items-center justify-end gap-x-2 text-[11px] tabular-nums text-muted-foreground">
                                <span>↓ {{ stats.input_cost.formatted.value }}{{ stats.input_cost.formatted.unit }}</span>
                                <span>↑ {{ stats.output_cost.formatted.value }}{{ stats.output_cost.formatted.unit }}</span>
                            </span>
                        </dd>
                    </div>

                    <!-- 等待时间 -->
                    <div class="flex flex-col rounded-2xl border bg-card p-3">
                        <dt class="flex items-center gap-1.5 text-xs text-muted-foreground">
                            <LuClock class="size-3.5 text-primary" />
                            <span class="truncate">{{ t('avgWaitTime') }}</span>
                        </dt>
                        <dd class="mt-auto pt-2 text-right">
                            <span class="block text-lg font-bold tabular-nums text-card-foreground">
                                {{ stats.wait_time.formatted.value }}
                                <span v-if="stats.wait_time.formatted.unit" class="ml-0.5 text-xs font-normal text-muted-foreground">{{ stats.wait_time.formatted.unit }}</span>
                            </span>
                        </dd>
                    </div>
                </dl>
            </section>

            <!-- 右列: 模型统计, 仅列出当前维度下的前六名 -->
            <section class="flex flex-col gap-2">
                <div class="flex h-7 items-center justify-between gap-2">
                    <h4 class="flex items-center gap-2 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                        {{ t('models') }}
                        <span class="tabular-nums">({{ channel.models.length }})</span>
                    </h4>
                    <div class="flex shrink-0 items-center text-xs">
                        <template v-for="(item, index) in sortOptions" :key="item.key">
                            <span v-if="index > 0" aria-hidden="true" class="mx-1 text-muted-foreground/40">/</span>
                            <button
                                type="button"
                                @click="modelSort = item.key"
                                :aria-pressed="modelSort === item.key"
                                :class="[
                                    'transition-colors',
                                    modelSort === item.key
                                        ? 'font-medium text-foreground'
                                        : 'text-muted-foreground/50 hover:text-muted-foreground'
                                ]"
                            >
                                {{ item.label }}
                            </button>
                        </template>
                    </div>
                </div>

                <div v-if="modelStats.length === 0" class="rounded-2xl border bg-card p-6 text-center text-xs text-muted-foreground">
                    {{ t('noModels') }}
                </div>
                <ul v-else class="space-y-2">
                    <li
                        v-for="model in modelStats.slice(0, 6)"
                        :key="model.id"
                        class="grid gap-2 rounded-2xl border bg-card p-3 @md/stats:grid-cols-[minmax(0,1fr)_auto] @md/stats:items-center @md/stats:gap-4 @2xl/stats:h-16"
                    >
                        <div class="min-w-0 space-y-1.5">
                            <span class="block truncate text-sm font-medium text-card-foreground">{{ model.name }}</span>
                            <div class="flex items-center gap-2">
                                <div class="h-1.5 flex-1 overflow-hidden rounded-full bg-muted">
                                    <div class="h-full rounded-full bg-primary" :style="{ width: `${model.share}%` }" />
                                </div>
                                <span class="w-8 shrink-0 text-right text-[10px] tabular-nums text-muted-foreground">
                                    {{ model.share.toFixed(0) }}%
                                </span>
                            </div>
                        </div>

                        <div class="grid grid-cols-2 gap-x-3 gap-y-1 text-xs tabular-nums @md/stats:w-64 @md/stats:grid-cols-4 @md/stats:gap-x-2">
                            <div class="flex items-center gap-1">
                                <LuMessageSquare class="size-3.5 shrink-0 text-chart-1" />
                                <span>
                                    {{ model.count.formatted.value }}
                                    <span v-if="model.count.formatted.unit" class="ml-0.5 text-xs font-normal text-muted-foreground">{{ model.count.formatted.unit }}</span>
                                </span>
                            </div>
                            <div class="flex items-center gap-1">
                                <LuCheckCircle2 class="size-3.5 shrink-0 text-accent" />
                                <span>{{ model.rate.toFixed(0) }}%</span>
                            </div>
                            <div class="flex items-center gap-1">
                                <LuFileText class="size-3.5 shrink-0 text-chart-3" />
                                <span>
                                    {{ model.tokens.formatted.value }}
                                    <span v-if="model.tokens.formatted.unit" class="ml-0.5 text-xs font-normal text-muted-foreground">{{ model.tokens.formatted.unit }}</span>
                                </span>
                            </div>
                            <div class="flex items-center gap-1">
                                <LuDollarSign class="size-3.5 shrink-0 text-chart-5" />
                                <span>
                                    {{ model.cost.formatted.value }}
                                    <span v-if="model.cost.formatted.unit" class="ml-0.5 text-xs font-normal text-muted-foreground">{{ model.cost.formatted.unit }}</span>
                                </span>
                            </div>
                        </div>
                    </li>
                </ul>
            </section>
        </div>

        <!-- 分享图预览: 覆盖整张弹窗卡片 -->
        <div v-if="preview" class="absolute inset-0 z-30 flex flex-col items-center justify-center gap-3 rounded-3xl bg-card/95 p-4 backdrop-blur-sm">
            <img
                :src="preview.url"
                alt=""
                class="max-h-[70vh] min-h-0 w-auto max-w-full rounded-2xl border border-border object-contain"
            />
            <div class="flex shrink-0 items-center gap-2">
                <button type="button" @click="handleCopyImage" :class="previewActionClass">
                    <LuCopy class="size-4" />
                </button>
                <button type="button" @click="handleDownloadImage" :class="previewActionClass">
                    <LuDownload class="size-4" />
                </button>
                <button type="button" @click="preview = null" :class="previewActionClass">
                    <LuX class="size-4" />
                </button>
            </div>
        </div>
    </div>
</template>
