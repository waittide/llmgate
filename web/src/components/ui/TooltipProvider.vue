<script setup lang="ts">
import { provide, toRef } from 'vue';

const props = withDefaults(
    defineProps<{
        delayDuration?: number;
        skipDelayDuration?: number;
        disableHoverableContent?: boolean;
    }>(),
    {
        delayDuration: 0,
        skipDelayDuration: 300,
        disableHoverableContent: false,
    }
);

provide('tooltipProviderContext', {
    delayDuration: toRef(props, 'delayDuration'),
    disableHoverableContent: toRef(props, 'disableHoverableContent'),
});
</script>

<template>
    <slot />
</template>
