<script setup lang="ts">
import { type CSSProperties, type HTMLAttributes } from 'vue';
import { motion } from 'motion-v';
import { cn } from '@/lib/utils';
import { useMorphingDialog } from './morphing-dialog';

export interface MorphingDialogSubtitleProps {
  class?: HTMLAttributes['class'];
  className?: string;
  style?: CSSProperties;
}

const props = defineProps<MorphingDialogSubtitleProps>();

const { uniqueId } = useMorphingDialog();
</script>

<template>
  <motion.div
    :layout-id="`dialog-subtitle-container-${uniqueId}`"
    :class="cn(props.class, className)"
    :style="props.style"
  >
    <slot />
  </motion.div>
</template>
