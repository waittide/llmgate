<script setup lang="ts">
import { inject, ref, onMounted, nextTick } from 'vue';
import type { PopoverContext } from './Popover.vue';

const props = withDefaults(
    defineProps<{
        asChild?: boolean;
        class?: any;
    }>(),
    {
        asChild: false,
    }
);

const popover = inject<PopoverContext>('popoverContext');
const anchorEl = ref<HTMLElement | null>(null);

const getTargetElement = (): HTMLElement | null => {
    if (!anchorEl.value) return null;
    if (props.asChild && anchorEl.value.firstElementChild instanceof HTMLElement) {
        return anchorEl.value.firstElementChild;
    }
    return anchorEl.value;
};

onMounted(async () => {
    await nextTick();
    if (popover) {
        popover.setTriggerRef(getTargetElement());
    }
});
</script>

<template>
    <div ref="anchorEl" data-slot="popover-anchor" class="inline-block" :class="props.class">
        <slot />
    </div>
</template>
