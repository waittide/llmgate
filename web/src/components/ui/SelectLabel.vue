<script setup lang="ts">
import type { HTMLAttributes } from 'vue';
import { cn } from '@/lib/utils';

const props = defineProps<{
    class?: HTMLAttributes['class'];
}>();
</script>

<template>
    <div data-slot="select-label" :class="cn('text-muted-foreground px-2 py-1.5 text-xs', props.class)">
        <slot />
    </div>
</template>
