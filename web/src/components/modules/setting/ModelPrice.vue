<script setup lang="ts">
import { ref, watch, onUnmounted } from 'vue';
import { LuClock, LuDatabaseBackup, LuDollarSign, LuRefreshCw } from 'vue-icons-plus/lu';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { useSettingList, useSetSetting, SettingKey } from '@/api/setting';
import { useRebuildModelPrice, useUpdateModelPrice, useLastUpdateTime } from '@/api/model';
import { useTranslations } from '@/composables/i18n';
import { toast } from '@/composables/toast';

const { t } = useTranslations('setting');
const { data: settings } = useSettingList();
const setSetting = useSetSetting();
const updatePrice = useUpdateModelPrice();
const rebuildPrice = useRebuildModelPrice();
const { data: lastUpdateTime } = useLastUpdateTime();

const updateInterval = ref('');
const confirmRebuild = ref(false);
const rebuildCountdown = ref(0);
const initialUpdateInterval = ref('');

watch(settings, (val) => {
    if (val) {
        const interval = val.find(s => s.key === SettingKey.ModelInfoUpdateInterval);
        if (interval) {
            updateInterval.value = interval.value;
            initialUpdateInterval.value = interval.value;
        }
    }
}, { immediate: true });

let rebuildTimer: number | null = null;

watch([confirmRebuild, rebuildCountdown], ([confirm, countdown]) => {
    if (rebuildTimer !== null) {
        window.clearTimeout(rebuildTimer);
        rebuildTimer = null;
    }
    if (confirm && countdown > 0) {
        rebuildTimer = window.setTimeout(() => {
            rebuildCountdown.value -= 1;
        }, 1000);
    }
});

onUnmounted(() => {
    if (rebuildTimer !== null) {
        window.clearTimeout(rebuildTimer);
    }
});

const handleSave = (key: string, value: string, initialValue: string) => {
    if (value === initialValue) return;

    setSetting.mutate({ key, value }, {
        onSuccess: () => {
            toast.success(t('saved'));
            initialUpdateInterval.value = value;
        }
    });
};

const handleManualUpdate = () => {
    updatePrice.mutate(undefined, {
        onSuccess: () => {
            toast.success(t('llmPrice.updateSuccess'));
        },
        onError: () => {
            toast.error(t('llmPrice.updateFailed'));
        }
    });
};

const handleRebuild = () => {
    if (rebuildCountdown.value > 0) return;
    rebuildPrice.mutate(undefined, {
        onSuccess: ({ count }) => {
            confirmRebuild.value = false;
            rebuildCountdown.value = 0;
            toast.success(t('llmPrice.rebuild.success', { count: String(count) }));
        },
        onError: () => {
            confirmRebuild.value = false;
            rebuildCountdown.value = 0;
            toast.error(t('llmPrice.rebuild.failed'));
        }
    });
};

const formatLastUpdateTime = (timeStr: string | undefined) => {
    if (!timeStr) return t('llmPrice.neverUpdated');
    const date = new Date(Number(timeStr) * 1000);
    if (date.getFullYear() === 1) return t('llmPrice.neverUpdated');
    return date.toLocaleString();
};
</script>

<template>
    <div class="rounded-3xl border border-border bg-card p-6 space-y-5">
        <h2 class="text-lg font-bold text-card-foreground flex items-center gap-2">
            <LuDollarSign class="h-5 w-5" />
            {{ t('llmPrice.title') }}
        </h2>

        <!-- 更新间隔 -->
        <div class="flex items-center justify-between gap-4">
            <div class="flex items-center gap-3">
                <LuClock class="h-5 w-5 text-muted-foreground" />
                <span class="text-sm font-medium">{{ t('llmPrice.updateInterval.label') }}</span>
            </div>
            <Input
                type="number"
                v-model="updateInterval"
                @blur="handleSave(SettingKey.ModelInfoUpdateInterval, updateInterval, initialUpdateInterval)"
                :placeholder="t('llmPrice.updateInterval.placeholder')"
                class="w-48 rounded-xl"
            />
        </div>

        <!-- 手动更新 -->
        <div class="flex items-center justify-between gap-4">
            <div class="flex flex-col gap-1">
                <div class="flex items-center gap-3">
                    <LuRefreshCw class="h-5 w-5 text-muted-foreground" />
                    <span class="text-sm font-medium">{{ t('llmPrice.manualUpdate.label') }}</span>
                </div>
                <span class="text-xs text-muted-foreground ml-8">
                    {{ t('llmPrice.lastUpdate') }}: {{ formatLastUpdateTime(lastUpdateTime) }}
                </span>
            </div>
            <Button
                variant="outline"
                size="sm"
                @click="handleManualUpdate"
                :disabled="updatePrice.isPending.value"
                class="rounded-xl"
            >
                {{ updatePrice.isPending.value ? t('llmPrice.manualUpdate.updating') : t('llmPrice.manualUpdate.button') }}
            </Button>
        </div>

        <!-- 重建价格数据 -->
        <div class="flex items-center justify-between gap-4">
            <div class="flex items-center gap-3">
                <LuDatabaseBackup class="h-5 w-5 text-muted-foreground" />
                <span class="text-sm font-medium">{{ t('llmPrice.rebuild.label') }}</span>
            </div>
            <template v-if="!confirmRebuild">
                <Button
                    variant="outline"
                    size="sm"
                    @click="() => {
                        rebuildCountdown = 3;
                        confirmRebuild = true;
                    }"
                    :disabled="rebuildPrice.isPending.value"
                    class="rounded-xl"
                >
                    {{ t('llmPrice.rebuild.button') }}
                </Button>
            </template>
            <template v-else>
                <div class="flex items-center gap-2">
                    <Button
                        variant="ghost"
                        size="sm"
                        @click="() => {
                            confirmRebuild = false;
                            rebuildCountdown = 0;
                        }"
                        :disabled="rebuildPrice.isPending.value"
                        class="rounded-xl"
                    >
                        {{ t('llmPrice.rebuild.cancel') }}
                    </Button>
                    <Tooltip :open="true">
                        <TooltipTrigger as-child>
                            <span class="inline-flex">
                                <Button
                                    variant="destructive"
                                    size="sm"
                                    @click="handleRebuild"
                                    :disabled="rebuildCountdown > 0 || rebuildPrice.isPending.value"
                                    class="min-w-24 rounded-xl"
                                >
                                    {{
                                        rebuildPrice.isPending.value
                                            ? t('llmPrice.rebuild.rebuilding')
                                            : rebuildCountdown > 0
                                                ? `${rebuildCountdown}s`
                                                : t('llmPrice.rebuild.confirm')
                                    }}
                                </Button>
                            </span>
                        </TooltipTrigger>
                        <TooltipContent side="top" :side-offset="10" align="center" class="whitespace-pre-line text-center">
                            {{ t('llmPrice.rebuild.description') }}
                        </TooltipContent>
                    </Tooltip>
                </div>
            </template>
        </div>
    </div>
</template>
