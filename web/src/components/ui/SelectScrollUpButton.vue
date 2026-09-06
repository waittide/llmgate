<script setup lang="ts">
import type { HTMLAttributes } from 'vue';
import { LuChevronUp } from 'vue-icons-plus/lu';
import { cn } from '@/lib/utils';

const props = defineProps<{
    class?: HTMLAttributes['class'];
}>();
</script>

<template>
    <div
        data-slot="select-scroll-up-button"
        aria-hidden="true"
        :class="cn('flex cursor-default items-center justify-center py-1', props.class)"
    >
        <LuChevronUp class="size-4" />
    </div>
</template>
