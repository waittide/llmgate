<script setup lang="ts">
import { ref, provide, watch, type Ref } from 'vue';

export interface PopoverContext {
    isOpen: Ref<boolean>;
    setOpen: (value: boolean) => void;
    triggerRef: Ref<HTMLElement | null>;
    contentRef: Ref<HTMLElement | null>;
    setTriggerRef: (el: HTMLElement | null) => void;
    setContentRef: (el: HTMLElement | null) => void;
}

const props = withDefaults(
    defineProps<{
        open?: boolean;
        defaultOpen?: boolean;
    }>(),
    {
        open: undefined,
        defaultOpen: false,
    }
);

const emit = defineEmits<{
    (e: 'update:open', value: boolean): void;
    (e: 'openChange', value: boolean): void;
}>();

const internalOpen = ref(props.open !== undefined ? props.open : props.defaultOpen);

watch(
    () => props.open,
    (val) => {
        if (val !== undefined) {
            internalOpen.value = val;
        }
    }
);

const isOpen = ref(internalOpen.value);

watch(
    () => [props.open, internalOpen.value],
    () => {
        isOpen.value = props.open !== undefined ? props.open : internalOpen.value;
    }
);

const setOpen = (val: boolean) => {
    if (props.open === undefined) {
        internalOpen.value = val;
    }
    emit('update:open', val);
    emit('openChange', val);
};

const triggerRef = ref<HTMLElement | null>(null);
const contentRef = ref<HTMLElement | null>(null);

const setTriggerRef = (el: HTMLElement | null) => {
    triggerRef.value = el;
};

const setContentRef = (el: HTMLElement | null) => {
    contentRef.value = el;
};

provide('popoverContext', {
    isOpen,
    setOpen,
    triggerRef,
    contentRef,
    setTriggerRef,
    setContentRef,
});
</script>

<template>
    <slot :open="isOpen" />
</template>
