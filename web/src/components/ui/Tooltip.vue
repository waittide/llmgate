<script setup lang="ts">
import { ref, provide, watch, inject, onBeforeUnmount, type Ref } from 'vue';

export interface TooltipContext {
    isOpen: Ref<boolean>;
    setOpen: (value: boolean) => void;
    show: () => void;
    hide: () => void;
    triggerRef: Ref<HTMLElement | null>;
    contentRef: Ref<HTMLElement | null>;
    setTriggerRef: (el: HTMLElement | null) => void;
    setContentRef: (el: HTMLElement | null) => void;
}

const props = withDefaults(
    defineProps<{
        open?: boolean;
        defaultOpen?: boolean;
        delayDuration?: number;
        disableHoverableContent?: boolean;
    }>(),
    {
        open: undefined,
        defaultOpen: false,
        delayDuration: undefined,
        disableHoverableContent: undefined,
    }
);

const emit = defineEmits<{
    (e: 'update:open', value: boolean): void;
    (e: 'openChange', value: boolean): void;
}>();

const providerContext = inject<{
    delayDuration: Ref<number>;
    disableHoverableContent: Ref<boolean>;
} | null>('tooltipProviderContext', null);

const effectiveDelay = () => {
    if (props.delayDuration !== undefined) return props.delayDuration;
    if (providerContext?.delayDuration.value !== undefined) return providerContext.delayDuration.value;
    return 0;
};

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

let timer: number | null = null;

const clearTimer = () => {
    if (timer !== null) {
        window.clearTimeout(timer);
        timer = null;
    }
};

const setOpen = (val: boolean) => {
    clearTimer();
    if (props.open === undefined) {
        internalOpen.value = val;
    }
    emit('update:open', val);
    emit('openChange', val);
};

const show = () => {
    clearTimer();
    const delay = effectiveDelay();
    if (delay <= 0) {
        setOpen(true);
    } else {
        timer = window.setTimeout(() => {
            setOpen(true);
        }, delay);
    }
};

const hide = () => {
    clearTimer();
    setOpen(false);
};

onBeforeUnmount(() => {
    clearTimer();
});

const triggerRef = ref<HTMLElement | null>(null);
const contentRef = ref<HTMLElement | null>(null);

const setTriggerRef = (el: HTMLElement | null) => {
    triggerRef.value = el;
};

const setContentRef = (el: HTMLElement | null) => {
    contentRef.value = el;
};

provide('tooltipContext', {
    isOpen,
    setOpen,
    show,
    hide,
    triggerRef,
    contentRef,
    setTriggerRef,
    setContentRef,
});
</script>

<template>
    <slot :open="isOpen" />
</template>
