<script setup lang="ts">
import { computed, ref, type CSSProperties } from 'vue';
import { LuCopy, LuCheck } from 'vue-icons-plus/lu';
import { useTranslations } from '@/composables/i18n';
import { toast } from '@/composables/toast';
import { cn } from '@/lib/utils';
import JsonNode from './JsonNode.vue';

export interface JsonViewProps {
    value: any;
    collapsed?: boolean | number;
    displayDataTypes?: boolean;
    displayObjectSize?: boolean;
    enableClipboard?: boolean;
    copyable?: boolean;
    indent?: number;
    style?: CSSProperties;
    class?: any;
    className?: any;
}

const props = withDefaults(defineProps<JsonViewProps>(), {
    collapsed: false,
    displayDataTypes: false,
    displayObjectSize: false,
    enableClipboard: true,
    copyable: true,
    indent: 2,
    style: () => ({}),
    class: undefined,
    className: undefined,
});

const { t } = useTranslations('common.copy');
const copiedAll = ref(false);
let copyAllTimer: number | null = null;

const parsedData = computed(() => {
    if (typeof props.value === 'string') {
        try {
            return JSON.parse(props.value);
        } catch {
            return props.value;
        }
    }
    return props.value;
});

async function copyAll() {
    try {
        const text = typeof props.value === 'string'
            ? props.value
            : JSON.stringify(props.value, null, props.indent);
        if (navigator.clipboard && window.isSecureContext) {
            await navigator.clipboard.writeText(text);
        } else {
            const textArea = document.createElement('textarea');
            textArea.value = text;
            textArea.style.position = 'fixed';
            textArea.style.left = '-999999px';
            document.body.appendChild(textArea);
            textArea.focus();
            textArea.select();
            document.execCommand('copy');
            textArea.remove();
        }
        copiedAll.value = true;
        toast.success(t('success') || 'Copied');
        if (copyAllTimer) window.clearTimeout(copyAllTimer);
        copyAllTimer = window.setTimeout(() => {
            copiedAll.value = false;
        }, 2000);
    } catch {
        toast.error(t('failed') || 'Copy failed');
    }
}
</script>

<template>
    <div
        :class="cn('relative font-mono text-xs rounded-lg p-2 overflow-auto bg-transparent', props.class, props.className)"
        :style="props.style"
    >
        <!-- 右上角快捷复制全局内容 -->
        <div v-if="enableClipboard && copyable" class="absolute right-2 top-2 z-10">
            <button
                type="button"
                @click="copyAll"
                class="inline-flex items-center gap-1 px-2 py-1 rounded bg-muted/60 hover:bg-muted text-[11px] text-muted-foreground hover:text-foreground transition-colors border border-border/40 shadow-xs"
                title="Copy entire JSON"
            >
                <LuCheck v-if="copiedAll" class="size-3 text-emerald-500" />
                <LuCopy v-else class="size-3" />
                <span>{{ copiedAll ? (t('success') || 'Copied') : 'JSON' }}</span>
            </button>
        </div>

        <JsonNode
            :data="parsedData"
            :collapsed="collapsed"
            :display-data-types="displayDataTypes"
            :display-object-size="displayObjectSize"
            :enable-clipboard="enableClipboard"
            :depth="0"
            :is-last="true"
        />
    </div>
</template>
