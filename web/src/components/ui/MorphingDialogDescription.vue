<script setup lang="ts">
import { type HTMLAttributes } from 'vue';
import { motion } from 'motion-v';
import { cn } from '@/lib/utils';
import { useMorphingDialog } from './morphing-dialog';

export interface MorphingDialogDescriptionProps {
  class?: HTMLAttributes['class'];
  className?: string;
  disableLayoutAnimation?: boolean;
  variants?: {
    initial?: Record<string, any> | string;
    animate?: Record<string, any> | string;
    exit?: Record<string, any> | string;
    [k: string]: any;
  };
}

const props = defineProps<MorphingDialogDescriptionProps>();

const { uniqueId } = useMorphingDialog();
</script>

<template>
  <motion.div
    :key="`dialog-description-${uniqueId}`"
    :layout-id="disableLayoutAnimation ? undefined : `dialog-description-content-${uniqueId}`"
    :variants="variants"
    :class="cn(props.class, className)"
    initial="initial"
    animate="animate"
    exit="exit"
    :id="`dialog-description-${uniqueId}`"
  >
    <slot />
  </motion.div>
</template>
