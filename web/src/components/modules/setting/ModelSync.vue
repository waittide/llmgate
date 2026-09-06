<script setup lang="ts">
import { ref, watch } from 'vue';
import { LuRefreshCw, LuClock } from 'vue-icons-plus/lu';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { useSettingList, useSetSetting, SettingKey } from '@/api/setting';
import { useLastSyncTime, useSyncChannel } from '@/api/channel';
import { useTranslations } from '@/composables/i18n';
import { toast } from '@/composables/toast';

const { t } = useTranslations('setting');
const { data: settings } = useSettingList();
const setSetting = useSetSetting();
const syncChannel = useSyncChannel();
const { data: lastSyncTime } = useLastSyncTime();

const syncInterval = ref('');
const initialSyncInterval = ref('');

watch(settings, (val) => {
    if (val) {
        const interval = val.find(s => s.key === SettingKey.SyncModelInterval);
        if (interval) {
            syncInterval.value = interval.value;
            initialSyncInterval.value = interval.value;
        }
    }
}, { immediate: true });

const handleSave = (key: string, value: string, initialValue: string) => {
    if (value === initialValue) return;

    setSetting.mutate({ key, value }, {
        onSuccess: () => {
            toast.success(t('saved'));
            initialSyncInterval.value = value;
        }
    });
};

const handleManualSync = () => {
    syncChannel.mutate(undefined, {
        onSuccess: () => {
            toast.success(t('modelSync.syncSuccess'));
        },
        onError: () => {
            toast.error(t('modelSync.syncFailed'));
        }
    });
};

const formatLastSyncTime = (timeStr: string | undefined) => {
    if (!timeStr) return t('modelSync.neverSynced');
    const date = new Date(Number(timeStr) * 1000);
    if (date.getFullYear() === 1) return t('modelSync.neverSynced');
    return date.toLocaleString();
};
</script>

<template>
    <div class="rounded-3xl border border-border bg-card p-6 space-y-5">
        <h2 class="text-lg font-bold text-card-foreground flex items-center gap-2">
            <LuRefreshCw class="h-5 w-5" />
            {{ t('modelSync.title') }}
        </h2>

        <!-- 同步间隔 -->
        <div class="flex items-center justify-between gap-4">
            <div class="flex items-center gap-3">
                <LuClock class="h-5 w-5 text-muted-foreground" />
                <span class="text-sm font-medium">{{ t('modelSync.syncInterval.label') }}</span>
            </div>
            <Input
                type="number"
                v-model="syncInterval"
                @blur="handleSave(SettingKey.SyncModelInterval, syncInterval, initialSyncInterval)"
                :placeholder="t('modelSync.syncInterval.placeholder')"
                class="w-48 rounded-xl"
            />
        </div>

        <!-- 手动同步 -->
        <div class="flex items-center justify-between gap-4">
            <div class="flex flex-col gap-1">
                <div class="flex items-center gap-3">
                    <LuRefreshCw class="h-5 w-5 text-muted-foreground" />
                    <span class="text-sm font-medium">{{ t('modelSync.manualSync.label') }}</span>
                </div>
                <span class="text-xs text-muted-foreground ml-8">
                    {{ t('modelSync.lastSync') }}: {{ formatLastSyncTime(lastSyncTime) }}
                </span>
            </div>
            <Button
                variant="outline"
                size="sm"
                @click="handleManualSync"
                :disabled="syncChannel.isPending.value"
                class="rounded-xl"
            >
                {{ syncChannel.isPending.value ? t('modelSync.manualSync.syncing') : t('modelSync.manualSync.button') }}
            </Button>
        </div>
    </div>
</template>
