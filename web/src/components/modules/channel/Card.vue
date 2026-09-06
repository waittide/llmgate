<script setup lang="ts">
import { ref, computed } from 'vue';
import {
    MorphingDialog,
    MorphingDialogTrigger,
    MorphingDialogContainer,
    MorphingDialogContent,
    MorphingDialogDescription,
} from '@/components/ui/morphing-dialog';
import {
    LuCheckCircle2,
    LuCheck,
    LuDollarSign,
    LuLayers,
    LuMessageSquare,
    LuPencil,
    LuTrash2,
    LuX,
    LuXCircle,
} from 'vue-icons-plus/lu';
import type { StatsMetricsFormatted } from '@/api/stats';
import { type Channel, useEnableChannel, useDeleteChannel } from '@/api/channel';
import ChannelStats from './Stats.vue';
import ChannelForm from './Form.vue';
import { useTranslations } from '@/composables/i18n';
import { Tooltip, TooltipTrigger, TooltipContent } from '@/components/ui/tooltip';
import { Switch } from '@/components/ui/switch';
import { toast } from '@/composables/toast';

interface Props {
    channel: Channel;
    stats: StatsMetricsFormatted;
    layout?: 'grid' | 'list';
}

const props = withDefaults(defineProps<Props>(), {
    layout: 'grid',
});

const { t } = useTranslations('channel.card');
const enableChannel = useEnableChannel();
const deleteChannel = useDeleteChannel();

// 点编辑图标时弹窗直接展示表单, 点卡片其余区域则展示统计; 卡片的捕获阶段先复位, 再由编辑按钮置位
const openInEditing = ref(false);
// 删除需二次确认, 确认态下图标常驻不隐藏
const isConfirmingDelete = ref(false);

const hoverActionClass =
    'flex size-8 items-center justify-center rounded-lg transition-all active:scale-95 sm:opacity-0 sm:group-hover:opacity-100 sm:focus-visible:opacity-100';

const handleEnableChange = (checked?: boolean) => {
    const isChecked = Boolean(checked);
    enableChannel.mutate(
        { id: props.channel.id, enabled: isChecked },
        {
            onSuccess: () => {
                toast.success(isChecked ? t('toast.enabled') : t('toast.disabled'));
            },
            onError: (error) => {
                toast.error(error.message);
            },
        }
    );
};

// 列表布局的统计项, 缺少 unit 时只显示数值
const listMetrics = computed(() => [
    {
        icon: LuMessageSquare,
        iconClass: 'size-3.5 text-primary',
        label: t('requestCount'),
        value: props.stats.request_count.formatted.value,
        unit: props.stats.request_count.formatted.unit,
    },
    {
        icon: LuLayers,
        iconClass: 'size-3.5 text-primary',
        label: t('model'),
        value: props.channel.models.length,
        unit: undefined,
    },
    {
        icon: LuCheckCircle2,
        iconClass: 'size-3.5 text-emerald-500',
        label: t('successRequests'),
        value: props.stats.request_success.formatted.value,
        unit: undefined,
    },
    {
        icon: LuXCircle,
        iconClass: 'size-3.5 text-destructive',
        label: t('failedRequests'),
        value: props.stats.request_failed.formatted.value,
        unit: undefined,
    },
    {
        icon: LuDollarSign,
        iconClass: 'size-3.5 text-primary',
        label: t('totalCost'),
        value: props.stats.total_cost.formatted.value,
        unit: props.stats.total_cost.formatted.unit,
    },
]);

// 网格布局的统计项
const gridMetrics = computed(() => [
    { icon: LuMessageSquare, label: t('requestCount'), metric: props.stats.request_count },
    { icon: LuDollarSign, label: t('totalCost'), metric: props.stats.total_cost },
]);
</script>

<template>
    <MorphingDialog>
        <MorphingDialogTrigger class="w-full text-left">
            <article
                @click.capture="openInEditing = false"
                class="group flex flex-col gap-4 rounded-3xl border border-border bg-card text-card-foreground p-4"
            >
                <header class="relative flex items-center justify-between gap-2">
                    <Tooltip>
                        <TooltipTrigger as-child>
                            <h3 class="text-lg font-bold truncate min-w-0">{{ channel.name }}</h3>
                        </TooltipTrigger>
                        <TooltipContent :key="channel.name" side="top" :side-offset="10" align="center">
                            {{ channel.name }}
                        </TooltipContent>
                    </Tooltip>
                    <div class="flex shrink-0 items-center gap-1">
                        <template v-if="isConfirmingDelete">
                            <button
                                type="button"
                                @click.stop="isConfirmingDelete = false"
                                :title="t('cancel')"
                                :aria-label="t('cancel')"
                                class="flex size-8 items-center justify-center rounded-lg text-muted-foreground transition-all hover:text-foreground active:scale-95"
                            >
                                <LuX class="size-4" />
                            </button>
                            <button
                                type="button"
                                @click.stop="deleteChannel.mutate(channel.id)"
                                :disabled="deleteChannel.isPending.value"
                                :title="t('confirmDelete')"
                                :aria-label="t('confirmDelete')"
                                class="flex size-8 items-center justify-center rounded-lg text-destructive transition-all hover:text-destructive/70 active:scale-95 disabled:opacity-50"
                            >
                                <LuCheck class="size-4" />
                            </button>
                        </template>
                        <template v-else>
                            <button
                                type="button"
                                @click.stop="isConfirmingDelete = true"
                                :title="t('delete')"
                                :aria-label="t('delete')"
                                :class="`${hoverActionClass} text-destructive hover:text-destructive/70`"
                            >
                                <LuTrash2 class="size-4" />
                            </button>
                            <button
                                type="button"
                                @click="openInEditing = true"
                                :title="t('edit')"
                                :aria-label="t('edit')"
                                :class="`${hoverActionClass} text-muted-foreground hover:text-foreground`"
                            >
                                <LuPencil class="size-4" />
                            </button>
                        </template>
                        <Switch
                            :model-value="channel.enabled"
                            @update:model-value="handleEnableChange"
                            :disabled="enableChannel.isPending.value"
                            @click.stop
                        />
                    </div>
                </header>

                <dl v-if="layout === 'list'" class="grid grid-cols-2 gap-2 lg:grid-cols-5">
                    <div
                        v-for="item in listMetrics"
                        :key="item.label"
                        class="rounded-2xl border border-border/70 bg-background/80 p-2"
                    >
                        <dt class="mb-1 flex items-center gap-1 text-xs text-muted-foreground">
                            <component :is="item.icon" :class="item.iconClass" />
                            {{ item.label }}
                        </dt>
                        <dd class="text-sm font-semibold">
                            {{ item.value }}
                            <span v-if="item.unit" class="ml-1 text-xs text-muted-foreground">{{ item.unit }}</span>
                        </dd>
                    </div>
                </dl>

                <dl v-else class="grid grid-cols-1 gap-3">
                    <div
                        v-for="item in gridMetrics"
                        :key="item.label"
                        class="flex items-center justify-between rounded-2xl border border-border/70 bg-background/80 p-2"
                    >
                        <div class="flex items-center gap-3">
                            <span class="flex h-10 w-10 items-center justify-center rounded-lg bg-primary/10 text-primary">
                                <component :is="item.icon" class="h-5 w-5" />
                            </span>
                            <dt class="text-sm text-muted-foreground">{{ item.label }}</dt>
                        </div>
                        <dd class="text-base">
                            {{ item.metric.formatted.value }}
                            <span class="ml-1 text-xs text-muted-foreground">{{ item.metric.formatted.unit }}</span>
                        </dd>
                    </div>
                </dl>
            </article>
        </MorphingDialogTrigger>

        <MorphingDialogContainer>
            <MorphingDialogContent class="relative w-full md:max-w-3xl bg-card text-card-foreground px-4 py-2 rounded-3xl max-h-[90vh] overflow-y-auto">
                <MorphingDialogDescription>
                    <ChannelForm v-if="openInEditing" :channel="channel" />
                    <ChannelStats v-else :channel="channel" :stats="stats" />
                </MorphingDialogDescription>
            </MorphingDialogContent>
        </MorphingDialogContainer>
    </MorphingDialog>
</template>
