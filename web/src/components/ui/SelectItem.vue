<script setup lang="ts">
import { inject, ref, computed, onMounted, onBeforeUnmount, type HTMLAttributes } from 'vue';
import { LuCheck } from 'vue-icons-plus/lu';
import { cn } from '@/lib/utils';
import type { SelectContext } from './Select.vue';

defineOptions({
    inheritAttrs: false,
});

const props = withDefaults(
    defineProps<{
        value: string;
        disabled?: boolean;
        class?: HTMLAttributes['class'];
    }>(),
    {
        disabled: false,
        class: undefined,
    }
);

const select = inject<SelectContext>('selectContext');
const itemRef = ref<HTMLDivElement | null>(null);

const isSelected = computed(() => select?.selectedValue.value === props.value);
const isHighlighted = computed(() => select?.highlightedValue.value === props.value);

const getItemLabel = (): string => {
    if (!itemRef.value) return props.value;
    return itemRef.value.textContent?.trim() || props.value;
};

const handleMouseEnter = () => {
    if (props.disabled || !select) return;
    select.setHighlightedValue(props.value);
};

const handleClick = () => {
    if (props.disabled || !select) return;
    select.selectValue(props.value);
};

onMounted(() => {
    if (select) {
        select.registerItem({
            value: props.value,
            label: getItemLabel(),
            disabled: props.disabled,
            el: itemRef.value,
        });
    }
});

onBeforeUnmount(() => {
    if (select) {
        select.unregisterItem(props.value);
    }
});
</script>

<template>
    <div
        ref="itemRef"
        role="option"
        tabindex="-1"
        :data-disabled="disabled ? '' : undefined"
        :data-highlighted="isHighlighted ? '' : undefined"
        :data-state="isSelected ? 'checked' : 'unchecked'"
        :aria-selected="isSelected ? 'true' : 'false'"
        :aria-disabled="disabled ? 'true' : 'false'"
        data-slot="select-item"
        :class="
            cn(
                'focus:bg-accent focus:text-accent-foreground [&_svg:not([class*=\'text-\'])]:text-muted-foreground relative flex w-full cursor-default items-center gap-2 rounded-sm py-1.5 pr-8 pl-2 text-sm outline-hidden select-none data-[disabled]:pointer-events-none data-[disabled]:opacity-50 data-[highlighted]:bg-accent data-[highlighted]:text-accent-foreground [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*=\'size-\'])]:size-4 *:[span]:last:flex *:[span]:last:items-center *:[span]:last:gap-2',
                props.class
            )
        "
        v-bind="$attrs"
        @mouseenter="handleMouseEnter"
        @mousemove="handleMouseEnter"
        @click="handleClick"
    >
        <span data-slot="select-item-indicator" class="absolute right-2 flex size-3.5 items-center justify-center">
            <LuCheck v-if="isSelected" class="size-4" />
        </span>
        <span data-slot="select-item-text" class="flex items-center gap-2">
            <slot />
        </span>
    </div>
</template>
