<script setup lang="ts">
import { inject, ref, onMounted, useAttrs, nextTick } from 'vue';
import type { TooltipContext } from './Tooltip.vue';

defineOptions({
    inheritAttrs: false,
});

const props = withDefaults(
    defineProps<{
        asChild?: boolean;
    }>(),
    {
        asChild: false,
    }
);

const attrs = useAttrs();
const tooltip = inject<TooltipContext>('tooltipContext');
const triggerEl = ref<HTMLElement | null>(null);

const getTargetElement = (): HTMLElement | null => {
    if (!triggerEl.value) return null;
    if (props.asChild && triggerEl.value.firstElementChild instanceof HTMLElement) {
        return triggerEl.value.firstElementChild;
    }
    return triggerEl.value;
};

const handleMouseEnter = () => {
    tooltip?.show();
};

const handleMouseLeave = () => {
    tooltip?.hide();
};

const handleFocus = () => {
    tooltip?.show();
};

const handleBlur = () => {
    tooltip?.hide();
};

onMounted(async () => {
    await nextTick();
    if (tooltip) {
        tooltip.setTriggerRef(getTargetElement());
    }
});
</script>

<template>
    <div
        ref="triggerEl"
        data-slot="tooltip-trigger"
        :data-state="tooltip?.isOpen.value ? 'open' : 'closed'"
        class="inline-block"
        v-bind="attrs"
        @mouseenter="handleMouseEnter"
        @mouseleave="handleMouseLeave"
        @focus="handleFocus"
        @blur="handleBlur"
    >
        <slot />
    </div>
</template>
