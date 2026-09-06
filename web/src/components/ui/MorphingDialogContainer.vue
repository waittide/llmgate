<script setup lang="ts">
import { ref, watch, onMounted, type CSSProperties, type HTMLAttributes } from 'vue';
import { motion, AnimatePresence } from 'motion-v';
import { cn } from '@/lib/utils';
import { useMorphingDialog } from './morphing-dialog';

export interface MorphingDialogContainerProps {
  class?: HTMLAttributes['class'];
  className?: string;
  style?: CSSProperties;
}

const props = defineProps<MorphingDialogContainerProps>();

const { isOpen, uniqueId } = useMorphingDialog();
const mounted = ref(false);
const hasOpened = ref(false);

onMounted(() => {
  mounted.value = true;
});

watch(
  isOpen,
  (val) => {
    if (val && !hasOpened.value) {
      hasOpened.value = true;
    }
  },
  { immediate: true }
);
</script>

<template>
  <Teleport to="body" v-if="mounted && (isOpen || hasOpened)">
    <AnimatePresence :initial="false" mode="sync">
      <template v-if="isOpen">
        <motion.div
          :key="`backdrop-${uniqueId}`"
          class="fixed inset-0 h-full w-full bg-white/40 backdrop-blur-xs dark:bg-black/40 z-50"
          :initial="{ opacity: 0 }"
          :animate="{ opacity: 1 }"
          :exit="{ opacity: 0 }"
        />
        <div
          :class="cn('fixed inset-0 z-50 flex items-center justify-center pointer-events-none [&>*]:pointer-events-auto', props.class, className)"
          :style="props.style"
        >
          <slot />
        </div>
      </template>
    </AnimatePresence>
  </Teleport>
</template>
