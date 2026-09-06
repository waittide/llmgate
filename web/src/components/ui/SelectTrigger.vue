<script setup lang="ts">
import { inject, ref, onMounted, type HTMLAttributes } from 'vue';
import { LuChevronDown } from 'vue-icons-plus/lu';
import { cn } from '@/lib/utils';
import type { SelectContext } from './Select.vue';

defineOptions({
    inheritAttrs: false,
});

const props = withDefaults(
    defineProps<{
        size?: 'sm' | 'default';
        id?: string;
        disabled?: boolean;
        class?: HTMLAttributes['class'];
    }>(),
    {
        size: 'default',
        id: undefined,
        disabled: false,
        class: undefined,
    }
);

const select = inject<SelectContext>('selectContext');
const buttonRef = ref<HTMLButtonElement | null>(null);

const isDisabled = () => props.disabled || select?.disabled.value || false;

const handleClick = (e: MouseEvent) => {
    if (isDisabled() || !select) return;
    select.setOpen(!select.isOpen.value);
};

const handleKeyDown = (e: KeyboardEvent) => {
    if (isDisabled() || !select) return;

    if (e.key === ' ' || e.key === 'Enter' || e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        if (!select.isOpen.value) {
            select.setOpen(true);
        }
    }
};

onMounted(() => {
    if (buttonRef.value && select) {
        select.setTriggerRef(buttonRef.value);
    }
});
</script>

<template>
    <button
        ref="buttonRef"
        type="button"
        role="combobox"
        :id="id"
        :disabled="isDisabled()"
        data-slot="select-trigger"
        :data-size="size"
        :data-state="select?.isOpen.value ? 'open' : 'closed'"
        :aria-expanded="select?.isOpen.value ? 'true' : 'false'"
        aria-haspopup="listbox"
        :class="
            cn(
                'border-input data-[placeholder]:text-muted-foreground [&_svg:not([class*=\'text-\'])]:text-muted-foreground focus-visible:border-ring focus-visible:ring-ring/50 aria-invalid:ring-destructive/20 dark:aria-invalid:ring-destructive/40 aria-invalid:border-destructive dark:bg-input/30 dark:hover:bg-input/50 flex w-fit items-center justify-between gap-2 rounded-md border bg-transparent px-3 py-2 text-sm whitespace-nowrap shadow-xs transition-[color,box-shadow] outline-none focus-visible:ring-[3px] disabled:cursor-not-allowed disabled:opacity-50 data-[size=default]:h-9 data-[size=sm]:h-8 *:data-[slot=select-value]:line-clamp-1 *:data-[slot=select-value]:flex *:data-[slot=select-value]:items-center *:data-[slot=select-value]:gap-2 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*=\'size-\'])]:size-4',
                props.class
            )
        "
        v-bind="$attrs"
        @click="handleClick"
        @keydown="handleKeyDown"
    >
        <slot />
        <LuChevronDown class="size-4 opacity-50 shrink-0 pointer-events-none" />
    </button>
</template>
