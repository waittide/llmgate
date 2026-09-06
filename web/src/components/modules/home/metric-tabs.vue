<script setup lang="ts">
import { useTranslations } from '@/composables/i18n';
import { cn } from '@/lib/utils';
import type { MetricKey } from './store';

export interface MetricTabsProps {
    value: MetricKey;
}

const props = defineProps<MetricTabsProps>();
const emit = defineEmits<{
    (e: 'update:value', value: MetricKey): void;
    (e: 'change', value: MetricKey): void;
}>();

const { t } = useTranslations('home.metric');

const items: Array<[MetricKey, string]> = [
    ['cost', 'pr-0'],
    ['count', 'px-0'],
    ['tokens', 'pl-0'],
];

function select(key: MetricKey) {
    emit('update:value', key);
    emit('change', key);
}
</script>

<template>
    <div class="inline-flex items-center gap-0 p-0 text-sm font-medium">
        <template v-for="([key, padding], index) in items" :key="key">
            <span
                v-if="index > 0"
                aria-hidden="true"
                class="mx-1 inline-flex h-full -translate-y-px items-center text-sm font-medium leading-none text-muted-foreground/50 select-none"
            >/</span>
            <button
                type="button"
                @click="select(key)"
                :class="cn(
                    'cursor-pointer transition-colors py-0.5 text-xs font-medium focus:outline-none',
                    padding,
                    props.value === key
                        ? 'text-foreground font-semibold'
                        : 'text-muted-foreground hover:text-foreground'
                )"
            >
                {{ t(key) }}
            </button>
        </template>
    </div>
</template>
