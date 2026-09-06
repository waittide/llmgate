<script setup lang="ts">
import { type CSSProperties, type HTMLAttributes } from 'vue';
import { motion } from 'motion-v';
import { cn } from '@/lib/utils';
import { useMorphingDialog } from './morphing-dialog';

export interface MorphingDialogTitleProps {
  class?: HTMLAttributes['class'];
  className?: string;
  style?: CSSProperties;
}

const props = defineProps<MorphingDialogTitleProps>();

const { uniqueId } = useMorphingDialog();
</script>

<template>
  <motion.div
    :layout-id="`dialog-title-container-${uniqueId}`"
    :class="cn(props.class, className)"
    :style="props.style"
    layout
  >
    <slot />
  </motion.div>
</template>
