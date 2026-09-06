<script setup lang="ts">
import { type HTMLAttributes } from 'vue';
import { motion } from 'motion-v';
import { LuX } from 'vue-icons-plus/lu';
import { cn } from '@/lib/utils';
import { useMorphingDialog } from './morphing-dialog';

export interface MorphingDialogCloseProps {
  class?: HTMLAttributes['class'];
  className?: string;
  variants?: {
    initial?: Record<string, any> | string;
    animate?: Record<string, any> | string;
    exit?: Record<string, any> | string;
    [k: string]: any;
  };
}

const props = defineProps<MorphingDialogCloseProps>();

const { setIsOpen, uniqueId } = useMorphingDialog();

const handleClose = () => {
  setIsOpen(false);
};
</script>

<template>
  <motion.button
    type="button"
    aria-label="Close dialog"
    :key="`dialog-close-${uniqueId}`"
    :class="cn('absolute top-6 right-6', props.class, className)"
    initial="initial"
    animate="animate"
    exit="exit"
    :variants="variants"
    @click="handleClose"
  >
    <slot>
      <LuX :size="24" />
    </slot>
  </motion.button>
</template>
