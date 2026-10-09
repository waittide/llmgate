<script setup lang="ts">
import { computed } from 'vue';
import { LuLoader2 } from 'vue-icons-plus/lu';
import { useTranslations } from '@/composables/i18n';

interface Props {
    url?: string;
    headers?: string;
    /** 客户端原始请求头；与 headers 同时存在时两者并列展示。 */
    originHeaders?: string;
    /** headers 的标题，缺省为通用 Header；请求侧传「上游请求头」以区分来源。 */
    headersLabel?: string;
    loading: boolean;
}

const props = defineProps<Props>();
const { t } = useTranslations('log.card');

function parseHeaders(raw?: string) {
    if (!raw) return null;
    try {
        return JSON.parse(raw) as object;
    } catch {
        return null;
    }
}

const parsedOriginHeaders = computed(() => parseHeaders(props.originHeaders));
const parsedHeaders = computed(() => parseHeaders(props.headers));
</script>

<template>
    <div
        v-if="url || headers || originHeaders"
        class="shrink-0 space-y-1.5 border-b border-border bg-muted/40 px-3 py-2"
    >
        <div v-if="url" class="break-all text-[11px] leading-relaxed text-muted-foreground">
            <span class="font-semibold text-foreground">{{ t('url') }}: </span>
            {{ url }}
        </div>
        <details v-if="parsedOriginHeaders" class="text-[11px]">
            <summary class="cursor-pointer select-none text-muted-foreground transition-colors hover:text-foreground">
                {{ t('originHeaders') }}
            </summary>
            <pre class="mt-1.5 whitespace-pre-wrap break-words font-mono leading-relaxed text-muted-foreground">{{ JSON.stringify(parsedOriginHeaders, null, 2) }}</pre>
        </details>
        <details v-if="parsedHeaders" class="text-[11px]">
            <summary class="cursor-pointer select-none text-muted-foreground transition-colors hover:text-foreground">
                {{ headersLabel ?? t('headers') }}
            </summary>
            <pre class="mt-1.5 whitespace-pre-wrap break-words font-mono leading-relaxed text-muted-foreground">{{ JSON.stringify(parsedHeaders, null, 2) }}</pre>
        </details>
        <LuLoader2 v-if="loading" class="size-3.5 animate-spin text-muted-foreground/60" />
    </div>
</template>
