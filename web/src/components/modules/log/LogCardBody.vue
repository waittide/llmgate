<script setup lang="ts">
import { computed, ref, watchEffect } from 'vue';
import { LuArrowRight, LuLoader2 } from 'vue-icons-plus/lu';
import { useTranslations } from '@/composables/i18n';
import { cn } from '@/lib/utils';
import type { RelayLogOverview } from '@/api/log';
import { Badge } from '@/components/ui/badge';
import {
    MorphingDialogTrigger,
    MorphingDialogContainer,
    useMorphingDialog,
} from '@/components/ui/morphing-dialog';
import { getModelIcon } from './model-icons';
import LogMetrics from './LogMetrics.vue';
import LogDetail from './LogDetail.vue';

interface Props {
    log: RelayLogOverview;
}

const props = defineProps<Props>();

const { t } = useTranslations('log.card');
const { isOpen } = useMorphingDialog();
const now = ref(Date.now());

const actualModel = computed(() => props.log.target_model || props.log.model);
const modelIconInfo = computed(() => getModelIcon(actualModel.value));
const requestRunning = computed(() => props.log.status === 'running' || props.log.status === 'committed');
const requestFailed = computed(() => props.log.status === 'failed' || props.log.status === 'canceled');
const requestClientClosed = computed(() => props.log.status === 'client_closed');
const errorText = computed(() => props.log.error ?? '');

let timer: number | null = null;
watchEffect((onCleanup) => {
    if (!requestRunning.value && !isOpen.value) return;
    timer = window.setInterval(() => {
        now.value = Date.now();
    }, 1000);
    onCleanup(() => {
        if (timer !== null) {
            window.clearInterval(timer);
            timer = null;
        }
    });
});
</script>

<template>
    <MorphingDialogTrigger
        :class="cn(
            'rounded-3xl border bg-card w-full text-left',
            requestFailed
                ? 'border-destructive/40'
                : requestClientClosed
                  ? 'border-orange-500/40'
                  : 'border-border',
        )"
    >
        <div :class="cn('p-4 grid grid-cols-[auto_1fr] gap-4', (requestFailed || (requestClientClosed && errorText)) ? 'items-start' : 'items-center')">
            <component
                :is="modelIconInfo.Icon"
                aria-hidden="true"
                :class="modelIconInfo.className"
                width="40"
                height="40"
            />
            <div class="min-w-0 flex flex-col gap-3">
                <div class="flex items-center gap-2 min-w-0 text-sm">
                    <span class="font-semibold text-card-foreground truncate" :title="log.model">
                        {{ log.model || t('unknownModel') }}
                    </span>
                    <LuLoader2
                        v-if="requestRunning"
                        class="size-3.5 shrink-0 animate-spin text-muted-foreground/50"
                    />
                    <LuArrowRight
                        v-else
                        class="size-3.5 shrink-0 text-muted-foreground/50"
                    />
                    <Badge
                        variant="secondary"
                        class="shrink-0 text-xs px-1.5 py-0"
                        :style="{ backgroundColor: `${modelIconInfo.color}15`, color: modelIconInfo.color }"
                    >
                        {{ log.target_channel || '-' }}
                    </Badge>
                    <span class="text-muted-foreground truncate" :title="actualModel">
                        {{ actualModel }}
                    </span>
                </div>
                <div class="grid grid-cols-12 gap-x-4 gap-y-2 text-xs tabular-nums text-muted-foreground md:grid-cols-7">
                    <LogMetrics :log="log" :now="now" :brand-color="modelIconInfo.color" variant="card" />
                </div>
                <div
                    v-if="requestFailed && errorText"
                    class="p-2.5 rounded-xl bg-destructive/10 border border-destructive/20 overflow-hidden"
                >
                    <p class="text-xs text-destructive line-clamp-2 whitespace-pre-line">{{ errorText }}</p>
                </div>
                <div
                    v-else-if="requestClientClosed && errorText"
                    class="p-2.5 rounded-xl bg-orange-500/10 border border-orange-500/20 overflow-hidden"
                >
                    <p class="text-xs text-orange-600 dark:text-orange-400 line-clamp-2 whitespace-pre-line">{{ errorText }}</p>
                </div>
            </div>
        </div>
    </MorphingDialogTrigger>

    <MorphingDialogContainer>
        <LogDetail :log="log" :now="now" />
    </MorphingDialogContainer>
</template>
