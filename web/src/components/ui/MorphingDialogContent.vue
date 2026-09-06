<script setup lang="ts">
import { ref, watch, onMounted, onBeforeUnmount, nextTick, type CSSProperties, type HTMLAttributes } from 'vue';
import { motion } from 'motion-v';
import { cn } from '@/lib/utils';
import { useMorphingDialog } from './morphing-dialog';

export interface MorphingDialogContentProps {
  class?: HTMLAttributes['class'];
  className?: string;
  style?: CSSProperties;
}

const props = defineProps<MorphingDialogContentProps>();

const { setIsOpen, isOpen, uniqueId, triggerRef } = useMorphingDialog();
const containerRef = ref<any>(null);
const firstFocusableElementRef = ref<HTMLElement | null>(null);
const lastFocusableElementRef = ref<HTMLElement | null>(null);

function getElement(target: any): HTMLElement | null {
  if (!target) return null;
  if (target instanceof HTMLElement) return target;
  if (target.$el instanceof HTMLElement) return target.$el;
  return null;
}

const handleKeyDown = (event: KeyboardEvent) => {
  if (!isOpen.value) return;

  if (event.key === 'Escape') {
    setIsOpen(false);
    return;
  }

  if (event.key === 'Tab') {
    if (!firstFocusableElementRef.value || !lastFocusableElementRef.value) return;

    if (event.shiftKey) {
      if (document.activeElement === firstFocusableElementRef.value) {
        event.preventDefault();
        lastFocusableElementRef.value.focus();
      }
    } else {
      if (document.activeElement === lastFocusableElementRef.value) {
        event.preventDefault();
        firstFocusableElementRef.value.focus();
      }
    }
  }
};

const handlePointerDown = (event: PointerEvent | MouseEvent) => {
  if (!isOpen.value) return;
  const target = event.target as HTMLElement | null;
  if (!target) return;

  const el = getElement(containerRef.value);
  if (el && el.contains(target)) {
    return;
  }

  // 点击 select-content 或 popover-content 时不触发外部关闭（与 React 原版一致）
  if (target.closest('[data-slot="select-content"]')) return;
  if (document.querySelector('[data-slot="select-content"]')) return;
  if (target.closest('[data-slot="popover-content"]')) return;
  if (document.querySelector('[data-slot="popover-content"]')) return;

  setIsOpen(false);
};

watch(
  isOpen,
  (open) => {
    if (typeof document === 'undefined') return;

    if (open) {
      document.body.classList.add('overflow-hidden');
      nextTick(() => {
        const el = getElement(containerRef.value);
        const focusableElements = el?.querySelectorAll<HTMLElement>(
          'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])'
        );
        if (focusableElements && focusableElements.length > 0) {
          firstFocusableElementRef.value = focusableElements[0];
          lastFocusableElementRef.value = focusableElements[focusableElements.length - 1];
          focusableElements[0].focus();
        }
      });
    } else {
      document.body.classList.remove('overflow-hidden');
      const triggerEl = getElement(triggerRef.value);
      triggerEl?.focus();
    }
  },
  { immediate: true }
);

onMounted(() => {
  document.addEventListener('keydown', handleKeyDown);
  document.addEventListener('pointerdown', handlePointerDown, true);
});

onBeforeUnmount(() => {
  document.removeEventListener('keydown', handleKeyDown);
  document.removeEventListener('pointerdown', handlePointerDown, true);
  if (typeof document !== 'undefined') {
    document.body.classList.remove('overflow-hidden');
  }
});
</script>

<template>
  <motion.div
    ref="containerRef"
    :layout-id="`dialog-${uniqueId}`"
    :class="cn('overflow-hidden', props.class, className)"
    :style="props.style"
    role="dialog"
    aria-modal="true"
    :aria-labelledby="`motion-ui-morphing-dialog-title-${uniqueId}`"
    :aria-describedby="`motion-ui-morphing-dialog-description-${uniqueId}`"
  >
    <slot />
  </motion.div>
</template>
