<script setup lang="ts">
import { inject, ref, onMounted, useAttrs, nextTick, type Ref } from 'vue';
import type { PopoverContext } from './Popover.vue';

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
const popover = inject<PopoverContext>('popoverContext');
const triggerEl = ref<HTMLElement | null>(null);

const getTargetElement = (): HTMLElement | null => {
    if (!triggerEl.value) return null;
    if (props.asChild && triggerEl.value.firstElementChild instanceof HTMLElement) {
        return triggerEl.value.firstElementChild;
    }
    return triggerEl.value;
};

const handleClick = () => {
    if (popover) {
        popover.setOpen(!popover.isOpen.value);
    }
};

onMounted(async () => {
    await nextTick();
    if (popover) {
        popover.setTriggerRef(getTargetElement());
    }
});
</script>

<template>
    <div
        ref="triggerEl"
        data-slot="popover-trigger"
        :data-state="popover?.isOpen.value ? 'open' : 'closed'"
        class="inline-block"
        v-bind="attrs"
        @click="handleClick"
    >
        <slot />
    </div>
</template>
