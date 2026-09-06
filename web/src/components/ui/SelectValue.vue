<script setup lang="ts">
import { inject, computed, type HTMLAttributes } from 'vue';
import { cn } from '@/lib/utils';
import type { SelectContext } from './Select.vue';

const props = withDefaults(
    defineProps<{
        placeholder?: string;
        class?: HTMLAttributes['class'];
    }>(),
    {
        placeholder: undefined,
        class: undefined,
    }
);

const select = inject<SelectContext>('selectContext');

const displayText = computed(() => {
    if (!select) return props.placeholder ?? '';
    const label = select.selectedLabel.value;
    if (label !== undefined && label !== '') return label;
    const value = select.selectedValue.value;
    if (value !== undefined && value !== '') return value;
    return props.placeholder ?? '';
});

const isPlaceholder = computed(() => {
    return !select?.selectedValue.value;
});
</script>

<template>
    <span
        data-slot="select-value"
        :data-placeholder="isPlaceholder ? '' : undefined"
        :class="cn('pointer-events-none truncate flex items-center gap-2', props.class)"
    >
        <slot>{{ displayText }}</slot>
    </span>
</template>
