<script setup lang="ts">
import { ref, computed, provide, useId } from 'vue';
import { MotionConfig } from 'motion-v';
import { MORPHING_DIALOG_KEY, type MorphingDialogProps } from './morphing-dialog-context';

const props = withDefaults(defineProps<MorphingDialogProps>(), {
  isOpen: undefined,
  open: undefined,
  modelValue: undefined,
  transition: undefined,
  defaultOpen: false,
});

const emit = defineEmits<{
  (e: 'update:isOpen', value: boolean): void;
  (e: 'update:open', value: boolean): void;
  (e: 'update:modelValue', value: boolean): void;
  (e: 'openChange', value: boolean): void;
}>();

const internalIsOpen = ref(props.defaultOpen);
const uniqueId = useId();
const triggerRef = ref<HTMLElement | null>(null);

const isOpen = computed({
  get() {
    if (props.isOpen !== undefined) return props.isOpen;
    if (props.open !== undefined) return props.open;
    if (props.modelValue !== undefined) return props.modelValue;
    return internalIsOpen.value;
  },
  set(val: boolean) {
    internalIsOpen.value = val;
    emit('update:isOpen', val);
    emit('update:open', val);
    emit('update:modelValue', val);
    emit('openChange', val);
  },
});

const setIsOpen = (value: boolean | ((prev: boolean) => boolean)) => {
  const nextVal = typeof value === 'function' ? value(isOpen.value) : value;
  isOpen.value = nextVal;
};

provide(MORPHING_DIALOG_KEY, {
  isOpen,
  setIsOpen,
  uniqueId,
  triggerRef,
});
</script>

<template>
  <MotionConfig :transition="transition">
    <slot />
  </MotionConfig>
</template>
