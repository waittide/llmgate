<script setup lang="ts">
import { computed, ref } from 'vue';
import { LuDatabase, LuDownload, LuUpload } from 'vue-icons-plus/lu';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { useExportDB, useImportDB } from '@/api/setting';
import { useTranslations } from '@/composables/i18n';
import { toast } from '@/composables/toast';

const { t } = useTranslations('setting');

const exportDB = useExportDB();
const importDB = useImportDB();

const file = ref<File | null>(null);
const fileInputRef = ref<HTMLInputElement | null>(null);

const rowsAffected = computed(() => importDB.data.value?.rows_affected ?? null);
const rowsAffectedList = computed(() => {
    if (!rowsAffected.value) return [];
    return Object.entries(rowsAffected.value)
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([k, v]) => ({ table: k, count: v }));
});

const onPickFile = (f: File | null) => {
    file.value = f;
};

const onImport = async () => {
    if (!file.value) {
        toast.error(t('backup.import.noFile'));
        return;
    }
    try {
        await importDB.mutateAsync(file.value);
        toast.success(t('backup.import.success'));
        if (fileInputRef.value) fileInputRef.value.value = '';
        file.value = null;
    } catch (e) {
        toast.error(e instanceof Error ? e.message : t('backup.import.failed'));
    }
};

const onExport = async () => {
    try {
        await exportDB.mutateAsync();
        toast.success(t('backup.export.success'));
    } catch (e) {
        toast.error(e instanceof Error ? e.message : t('backup.export.failed'));
    }
};
</script>

<template>
    <div class="rounded-3xl border border-border bg-card p-6 space-y-5">
        <h2 class="text-lg font-bold text-card-foreground flex items-center gap-2">
            <LuDatabase class="h-5 w-5" />
            {{ t('backup.title') }}
        </h2>

        <!-- 导出 -->
        <div>
            <Button
                type="button"
                variant="outline"
                class="w-full rounded-xl"
                @click="onExport"
                :disabled="exportDB.isPending.value"
            >
                <LuDownload class="size-4" />
                {{ exportDB.isPending.value ? t('backup.export.exporting') : t('backup.export.button') }}
            </Button>
        </div>

        <div class="h-px bg-border" />

        <!-- 导入 -->
        <div class="space-y-3">
            <div class="text-sm font-semibold text-card-foreground">{{ t('backup.import.title') }}</div>

            <Input
                ref="fileInputRef"
                type="file"
                accept="application/json,.json"
                @change="(e: Event) => onPickFile((e.target as HTMLInputElement).files?.[0] ?? null)"
                class="rounded-xl"
            />

            <Button
                type="button"
                variant="destructive"
                class="w-full rounded-xl"
                @click="onImport"
                :disabled="importDB.isPending.value"
            >
                <LuUpload class="size-4" />
                {{ importDB.isPending.value ? t('backup.import.importing') : t('backup.import.button') }}
            </Button>

            <div v-if="rowsAffectedList.length > 0" class="mt-2 space-y-1">
                <div class="text-xs font-semibold text-card-foreground">{{ t('backup.import.result') }}</div>
                <div class="grid grid-cols-2 gap-1 text-xs text-muted-foreground">
                    <div v-for="it in rowsAffectedList" :key="it.table" class="flex justify-between gap-2">
                        <span class="truncate">{{ it.table }}</span>
                        <span class="tabular-nums">{{ it.count }}</span>
                    </div>
                </div>
            </div>
        </div>
    </div>
</template>
