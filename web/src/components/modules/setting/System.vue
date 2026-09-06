<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { LuMonitor, LuClock, LuShield, LuHelpCircle, LuX } from 'vue-icons-plus/lu';
import { Input } from '@/components/ui/input';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { useSettingList, useSetSetting, SettingKey } from '@/api/setting';
import { useTranslations } from '@/composables/i18n';
import { toast } from '@/composables/toast';

const { t } = useTranslations('setting');
const { data: settings } = useSettingList();
const setSetting = useSetSetting();

const statsSaveInterval = ref('');
const corsAllowOrigins = ref('');
const corsInputValue = ref('');

const initialStatsSaveInterval = ref('');
const initialCorsAllowOrigins = ref('');

watch(settings, (val) => {
    if (val) {
        const interval = val.find(s => s.key === SettingKey.StatsSaveInterval);
        const cors = val.find(s => s.key === SettingKey.CORSAllowOrigins);
        if (interval) {
            statsSaveInterval.value = interval.value;
            initialStatsSaveInterval.value = interval.value;
        }
        if (cors) {
            corsAllowOrigins.value = cors.value;
            initialCorsAllowOrigins.value = cors.value;
        }
    }
}, { immediate: true });

const handleSave = (key: string, value: string, initialValue: string) => {
    if (value === initialValue) return;

    setSetting.mutate({ key, value }, {
        onSuccess: () => {
            toast.success(t('saved'));
            if (key === SettingKey.StatsSaveInterval) {
                initialStatsSaveInterval.value = value;
            } else if (key === SettingKey.CORSAllowOrigins) {
                initialCorsAllowOrigins.value = value;
            }
        }
    });
};

const corsAllowOriginsList = computed(() => {
    const value = corsAllowOrigins.value.trim();
    if (!value) return [];
    if (value === '*') return ['*'];
    return Array.from(new Set(
        value
            .split(/[,\n，]/)
            .map(item => item.trim())
            .filter(Boolean)
    ));
});

const corsAllowOriginsDisplay = computed(() => (
    corsAllowOriginsList.value.length > 0 ? corsAllowOriginsList.value.join(', ') : t('corsAllowOrigins.hint')
));

const saveCorsAllowOrigins = (origins: string[]) => {
    const normalizedOrigins = Array.from(new Set(
        origins
            .map(origin => origin.trim())
            .filter(Boolean)
    ));
    const normalizedValue = normalizedOrigins.includes('*') ? '*' : normalizedOrigins.join(',');
    corsAllowOrigins.value = normalizedValue;
    handleSave(SettingKey.CORSAllowOrigins, normalizedValue, initialCorsAllowOrigins.value);
};

const handleAddCorsOrigin = () => {
    const newOrigins = Array.from(new Set(
        corsInputValue.value
            .split(/[,\n，]/)
            .map(item => item.trim())
            .filter(Boolean)
    ));
    if (newOrigins.length === 0) return;

    if (newOrigins.includes('*')) {
        saveCorsAllowOrigins(['*']);
        corsInputValue.value = '';
        return;
    }

    const base = corsAllowOriginsList.value.includes('*') ? [] : corsAllowOriginsList.value;
    const merged = Array.from(new Set([...base, ...newOrigins]));
    saveCorsAllowOrigins(merged);
    corsInputValue.value = '';
};

const handleRemoveCorsOrigin = (originToRemove: string) => {
    const nextOrigins = corsAllowOriginsList.value.filter(origin => origin !== originToRemove);
    saveCorsAllowOrigins(nextOrigins);
};
</script>

<template>
    <div class="rounded-3xl border border-border bg-card p-6 space-y-5">
        <h2 class="text-lg font-bold text-card-foreground flex items-center gap-2">
            <LuMonitor class="h-5 w-5" />
            {{ t('system') }}
        </h2>

        <!-- 统计保存周期 -->
        <div class="flex items-center justify-between gap-4">
            <div class="flex items-center gap-3">
                <LuClock class="h-5 w-5 text-muted-foreground" />
                <span class="text-sm font-medium">{{ t('statsSaveInterval.label') }}</span>
            </div>
            <Input
                type="number"
                v-model="statsSaveInterval"
                @blur="handleSave('stats_save_interval', statsSaveInterval, initialStatsSaveInterval)"
                :placeholder="t('statsSaveInterval.placeholder')"
                class="w-48 rounded-xl"
            />
        </div>

        <!-- CORS 跨域白名单 -->
        <div class="flex items-center justify-between gap-4">
            <div class="flex items-center gap-3">
                <LuShield class="h-5 w-5 text-muted-foreground" />
                <span class="text-sm font-medium">{{ t('corsAllowOrigins.label') }}</span>
                <Tooltip>
                    <TooltipTrigger as-child>
                        <LuHelpCircle class="size-4 text-muted-foreground cursor-help" />
                    </TooltipTrigger>
                    <TooltipContent side="top" :side-offset="10" align="center">
                        {{ t('corsAllowOrigins.hint') }}
                        <br />
                        {{ t('corsAllowOrigins.example') }}
                    </TooltipContent>
                </Tooltip>
            </div>
            <Popover>
                <PopoverTrigger as-child>
                    <button
                        type="button"
                        class="border-input focus-visible:border-ring focus-visible:ring-ring/50 w-48 min-h-9 rounded-xl border bg-transparent px-3 py-2 text-left text-sm shadow-xs transition-[color,box-shadow] outline-none focus-visible:ring-[3px]"
                        :title="corsAllowOriginsDisplay"
                    >
                        <span :class="['block overflow-hidden text-ellipsis whitespace-nowrap', corsAllowOriginsList.length === 0 ? 'text-muted-foreground' : '']">
                            {{ corsAllowOriginsDisplay }}
                        </span>
                    </button>
                </PopoverTrigger>
                <PopoverContent class="w-72 space-y-2 rounded-3xl p-3 bg-card">
                    <Input
                        v-model="corsInputValue"
                        @keydown.enter.prevent="handleAddCorsOrigin"
                        :placeholder="t('corsAllowOrigins.example')"
                        class="h-9 rounded-xl"
                        autofocus
                    />
                    <div class="max-h-48 space-y-1 overflow-y-auto">
                        <template v-if="corsAllowOriginsList.length > 0">
                            <div
                                v-for="origin in corsAllowOriginsList"
                                :key="origin"
                                class="flex items-center justify-between gap-2 rounded-xl border border-border/60 px-2 py-1"
                            >
                                <span class="break-all text-xs leading-5">{{ origin }}</span>
                                <button
                                    type="button"
                                    @click="handleRemoveCorsOrigin(origin)"
                                    class="text-muted-foreground transition-colors hover:text-destructive"
                                    :aria-label="`remove ${origin}`"
                                >
                                    <LuX class="size-4" />
                                </button>
                            </div>
                        </template>
                    </div>
                </PopoverContent>
            </Popover>
        </div>
    </div>
</template>
