<script setup lang="ts">
import { computed } from 'vue';
import { LuLoader2 } from 'vue-icons-plus/lu';
import { useTranslations } from '@/composables/i18n';

interface Props {
    url?: string;
    headers?: string;
    loading: boolean;
}

const props = defineProps<Props>();
const { t } = useTranslations('log.card');

const parsedHeaders = computed(() => {
    if (!props.headers) return null;
    try {
        return JSON.parse(props.headers) as object;
    } catch {
        return null;
    }
});
</script>

<template>
    <div
        v-if="url || headers"
        class="shrink-0 space-y-1.5 border-b border-border bg-muted/40 px-3 py-2"
    >
        <div v-if="url" class="break-all text-[11px] leading-relaxed text-muted-foreground">
            <span class="font-semibold text-foreground">{{ t('url') }}: </span>
            {{ url }}
        </div>
        <details v-if="parsedHeaders" class="text-[11px]">
            <summary class="cursor-pointer select-none text-muted-foreground transition-colors hover:text-foreground">
                {{ t('headers') }}
            </summary>
            <pre class="mt-1.5 whitespace-pre-wrap break-words font-mono leading-relaxed text-muted-foreground">{{ JSON.stringify(parsedHeaders, null, 2) }}</pre>
        </details>
        <LuLoader2 v-if="loading" class="size-3.5 animate-spin text-muted-foreground/60" />
    </div>
</template>
