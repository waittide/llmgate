<script setup lang="ts">
import { type CSSProperties, type HTMLAttributes } from 'vue';
import { motion } from 'motion-v';
import { cn } from '@/lib/utils';
import { useMorphingDialog } from './morphing-dialog';

export interface MorphingDialogImageProps {
  src: string;
  alt?: string;
  class?: HTMLAttributes['class'];
  className?: string;
  style?: CSSProperties;
}

const props = withDefaults(defineProps<MorphingDialogImageProps>(), {
  alt: '',
});

const { uniqueId } = useMorphingDialog();
</script>

<template>
  <motion.img
    :src="props.src"
    :alt="props.alt"
    :class="cn(props.class, className)"
    :layout-id="`dialog-img-${uniqueId}`"
    :style="props.style"
  />
</template>
