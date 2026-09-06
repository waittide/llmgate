<script setup lang="ts">
import type { HTMLAttributes } from 'vue';
import { LuChevronDown } from 'vue-icons-plus/lu';
import { cn } from '@/lib/utils';

const props = defineProps<{
    class?: HTMLAttributes['class'];
}>();
</script>

<template>
    <div
        data-slot="select-scroll-down-button"
        aria-hidden="true"
        :class="cn('flex cursor-default items-center justify-center py-1', props.class)"
    >
        <LuChevronDown class="size-4" />
    </div>
</template>
