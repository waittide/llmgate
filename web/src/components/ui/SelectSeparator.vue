<script setup lang="ts">
import type { HTMLAttributes } from 'vue';
import { cn } from '@/lib/utils';

const props = defineProps<{
    class?: HTMLAttributes['class'];
}>();
</script>

<template>
    <div
        data-slot="select-separator"
        role="separator"
        :class="cn('bg-border pointer-events-none -mx-1 my-1 h-px', props.class)"
    />
</template>
