<script setup lang="ts">
import { ref, onUnmounted } from 'vue';
import { LuCheck, LuCopy } from 'vue-icons-plus/lu';
import { useTranslations } from '@/composables/i18n';
import { toast } from '@/composables/toast';
import { cn } from '@/lib/utils';

export interface CopyIconButtonProps {
    text: string;
    className?: string;
    copyIconClassName?: string;
    checkIconClassName?: string;
}

const props = defineProps<CopyIconButtonProps>();

const { t } = useTranslations('common.copy');
const copied = ref(false);
let timer: number | null = null;

onUnmounted(() => {
    if (timer !== null) {
        window.clearTimeout(timer);
    }
});

async function copyToClipboard(str: string): Promise<void> {
    if (navigator.clipboard && window.isSecureContext) {
        await navigator.clipboard.writeText(str);
        return;
    }
    const textArea = document.createElement('textarea');
    textArea.value = str;
    textArea.style.position = 'fixed';
    textArea.style.left = '-999999px';
    textArea.style.top = '-999999px';
    document.body.appendChild(textArea);
    textArea.focus();
    textArea.select();
    const successful = document.execCommand('copy');
    textArea.remove();
    if (!successful) {
        throw new Error('execCommand copy failed');
    }
}

async function handleClick() {
    if (!props.text) {
        toast.error(t('noContent') || t('failed'));
        return;
    }

    try {
        await copyToClipboard(props.text);
        copied.value = true;
        toast.success(t('success'));

        if (timer !== null) window.clearTimeout(timer);
        timer = window.setTimeout(() => {
            copied.value = false;
        }, 2000);
    } catch (err) {
        const description = err instanceof Error ? err.message : String(err);
        toast.error(`${t('failed')}: ${description}`);
    }
}
</script>

<template>
    <button
        type="button"
        @click="handleClick"
        aria-label="Copy"
        :class="cn(props.className)"
    >
        <span class="grid place-items-center">
            <LuCopy
                :class="cn(
                    'col-start-1 row-start-1 transition-transform duration-200 motion-reduce:transition-none',
                    copied ? 'scale-0 delay-0 ease-in' : 'scale-100 delay-200 ease-out',
                    props.copyIconClassName,
                )"
            />
            <LuCheck
                :class="cn(
                    'col-start-1 row-start-1 transition-transform duration-200 motion-reduce:transition-none',
                    copied ? 'scale-100 delay-200 ease-out' : 'scale-0 delay-0 ease-in',
                    props.checkIconClassName,
                )"
            />
        </span>
    </button>
</template>
