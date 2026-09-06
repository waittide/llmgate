<script setup lang="ts">
import { computed } from 'vue';
import { LuX } from 'vue-icons-plus/lu';
import { motion } from 'motion-v';
import type { APIKey } from '@/api/apikey';
import { useStatsAPIKey } from '@/api/stats';
import { useTranslations } from '@/composables/i18n';

const props = defineProps<{
    layoutId: string;
    apiKey: APIKey;
}>();

const emit = defineEmits<{
    (e: 'close'): void;
}>();

const { t } = useTranslations('setting');
const { data: statsList } = useStatsAPIKey();

const stats = computed(() => {
    const list = statsList.value ?? [];
    return list.find((s) => s.api_key_id === props.apiKey.id);
});
</script>

<template>
    <motion.div
        :layoutId="props.layoutId"
        class="absolute left-1/2 top-1/2 z-30 w-[min(320px,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 flex flex-col bg-card p-5 rounded-3xl border border-border max-h-[80vh] overflow-auto"
        :transition="{ type: 'spring', stiffness: 400, damping: 30 }"
    >
        <div class="flex items-center justify-between gap-2 mb-3">
            <h3 class="text-sm font-semibold text-card-foreground line-clamp-1">
                {{ props.apiKey.name }}
            </h3>
            <button
                type="button"
                @click="emit('close')"
                class="size-8 flex items-center justify-center rounded-lg bg-muted text-muted-foreground transition-colors hover:bg-muted/80"
            >
                <LuX class="size-4" />
            </button>
        </div>

        <div v-if="!stats" class="text-sm text-muted-foreground">{{ t('apiKey.stats.noData') }}</div>
        <div v-else class="grid grid-cols-2 gap-3 text-sm">
            <div class="rounded-lg bg-muted/40 p-3">
                <div class="text-xs text-muted-foreground">{{ t('apiKey.stats.inputToken') }}</div>
                <div class="font-medium tabular-nums">
                    {{ stats.input_token.formatted.value }}
                    {{ stats.input_token.formatted.unit }}
                </div>
            </div>
            <div class="rounded-lg bg-muted/40 p-3">
                <div class="text-xs text-muted-foreground">{{ t('apiKey.stats.outputToken') }}</div>
                <div class="font-medium tabular-nums">
                    {{ stats.output_token.formatted.value }}
                    {{ stats.output_token.formatted.unit }}
                </div>
            </div>
            <div class="rounded-lg bg-muted/40 p-3">
                <div class="text-xs text-muted-foreground">{{ t('apiKey.stats.inputCost') }}</div>
                <div class="font-medium tabular-nums">
                    {{ stats.input_cost.formatted.value }}
                    {{ stats.input_cost.formatted.unit }}
                </div>
            </div>
            <div class="rounded-lg bg-muted/40 p-3">
                <div class="text-xs text-muted-foreground">{{ t('apiKey.stats.outputCost') }}</div>
                <div class="font-medium tabular-nums">
                    {{ stats.output_cost.formatted.value }}
                    {{ stats.output_cost.formatted.unit }}
                </div>
            </div>
            <div class="rounded-lg bg-muted/40 p-3">
                <div class="text-xs text-muted-foreground">{{ t('apiKey.stats.requestSuccess') }}</div>
                <div class="font-medium tabular-nums">
                    {{ stats.request_success.formatted.value }}
                    {{ stats.request_success.formatted.unit }}
                </div>
            </div>
            <div class="rounded-lg bg-muted/40 p-3">
                <div class="text-xs text-muted-foreground">{{ t('apiKey.stats.requestFailed') }}</div>
                <div class="font-medium tabular-nums">
                    {{ stats.request_failed.formatted.value }}
                    {{ stats.request_failed.formatted.unit }}
                </div>
            </div>
        </div>
    </motion.div>
</template>
