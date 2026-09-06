<script setup lang="ts">
import { type CSSProperties, type HTMLAttributes } from 'vue';
import { motion } from 'motion-v';
import { cn } from '@/lib/utils';
import { useMorphingDialog } from './morphing-dialog';

export interface MorphingDialogTriggerProps {
  class?: HTMLAttributes['class'];
  className?: string;
  style?: CSSProperties;
}

const props = defineProps<MorphingDialogTriggerProps>();

const { setIsOpen, isOpen, uniqueId, triggerRef } = useMorphingDialog();

const handleClick = () => {
  setIsOpen(!isOpen.value);
};

const handleKeyDown = (event: KeyboardEvent) => {
  if (event.key === 'Enter' || event.key === ' ') {
    event.preventDefault();
    setIsOpen(!isOpen.value);
  }
};
</script>

<template>
  <div
    v-if="isOpen"
    ref="triggerRef"
    :class="cn('relative', props.class, className)"
    :style="{ ...props.style, visibility: 'hidden', pointerEvents: 'none' }"
    aria-hidden="true"
  >
    <slot />
  </div>
  <motion.div
    v-else
    ref="triggerRef"
    :layout-id="`dialog-${uniqueId}`"
    :class="cn('relative cursor-pointer', props.class, className)"
    :style="props.style"
    aria-haspopup="dialog"
    :aria-expanded="isOpen"
    :aria-controls="`motion-ui-morphing-dialog-content-${uniqueId}`"
    :aria-label="`Open dialog ${uniqueId}`"
    role="button"
    tabindex="0"
    @click="handleClick"
    @keydown="handleKeyDown"
  >
    <slot />
  </motion.div>
</template>
