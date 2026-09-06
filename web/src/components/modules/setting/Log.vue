<script setup lang="ts">
import { ref, watch } from 'vue';
import { LuCalendarClock, LuScrollText, LuTrash2 } from 'vue-icons-plus/lu';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { useClearLogs } from '@/api/log';
import { useSettingList, useSetSetting, SettingKey } from '@/api/setting';
import { useTranslations } from '@/composables/i18n';
import { toast } from '@/composables/toast';

const { t } = useTranslations('setting');
const clearLogs = useClearLogs();
const { data: settings } = useSettingList();
const setSetting = useSetSetting();

const retentionDays = ref('');
const initialRetention = ref('');

watch(settings, (val) => {
    if (!val) return;
    const retention = val.find((s) => s.key === SettingKey.LogRetentionDays);
    if (retention) {
        retentionDays.value = retention.value;
        initialRetention.value = retention.value;
    }
}, { immediate: true });

const handleSaveRetention = () => {
    const value = retentionDays.value.trim();
    if (value === initialRetention.value) return;
    const parsed = Number(value);
    if (Number.isNaN(parsed) || parsed < 1) {
        toast.error(t('log.retention.invalid'));
        retentionDays.value = initialRetention.value;
        return;
    }
    setSetting.mutate({ key: SettingKey.LogRetentionDays, value: String(Math.floor(parsed)) }, {
        onSuccess: () => {
            toast.success(t('saved'));
            initialRetention.value = String(Math.floor(parsed));
        },
        onError: () => {
            retentionDays.value = initialRetention.value;
            toast.error(t('log.retention.saveFailed'));
        },
    });
};

const handleClearLogs = () => {
    clearLogs.mutate(undefined, {
        onSuccess: () => toast.success(t('log.clearSuccess')),
        onError: () => toast.error(t('log.clearFailed')),
    });
};
</script>

<template>
    <div class="space-y-5 rounded-3xl border border-border bg-card p-6">
        <h2 class="flex items-center gap-2 text-lg font-bold text-card-foreground">
            <LuScrollText class="size-5" />
            {{ t('log.title') }}
        </h2>

        <!-- 日志保留天数 -->
        <div class="flex items-center justify-between gap-4">
            <div class="flex items-center gap-3">
                <LuCalendarClock class="size-5 text-muted-foreground" />
                <div class="flex flex-col">
                    <span class="text-sm font-medium">{{ t('log.retention.label') }}</span>
                    <span class="text-xs text-muted-foreground">{{ t('log.retention.hint') }}</span>
                </div>
            </div>
            <Input
                type="number"
                :min="1"
                v-model="retentionDays"
                @blur="handleSaveRetention"
                :placeholder="t('log.retention.placeholder')"
                class="w-32 rounded-xl"
            />
        </div>

        <!-- 清空日志 -->
        <div class="flex items-center justify-between gap-4">
            <div class="flex items-center gap-3">
                <LuTrash2 class="size-5 text-muted-foreground" />
                <span class="text-sm font-medium">{{ t('log.clear.label') }}</span>
            </div>
            <Button
                variant="destructive"
                size="sm"
                @click="handleClearLogs"
                :disabled="clearLogs.isPending.value"
                class="rounded-xl"
            >
                {{ clearLogs.isPending.value ? t('log.clear.clearing') : t('log.clear.button') }}
            </Button>
        </div>
    </div>
</template>
