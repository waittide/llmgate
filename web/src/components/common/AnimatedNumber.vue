<script setup lang="ts">
import { ref, watch, onUnmounted, computed } from 'vue';
import { animate } from 'motion-v';

export interface AnimatedNumberProps {
    value: string | number | undefined;
    duration?: number;
}

const props = withDefaults(defineProps<AnimatedNumberProps>(), {
    duration: 800,
});

const displayValue = ref(0);
const prevValue = ref(0);
let controls: { stop: () => void } | null = null;

watch(
    () => props.value,
    (newVal) => {
        if (controls) {
            controls.stop();
            controls = null;
        }

        if (newVal === undefined || newVal === null || newVal === '-') {
            prevValue.value = 0;
            displayValue.value = 0;
            return;
        }

        const numericValue = typeof newVal === 'string'
            ? parseFloat(newVal.replace(/,/g, ''))
            : newVal;

        if (Number.isNaN(numericValue)) {
            return;
        }

        try {
            controls = animate(prevValue.value, numericValue, {
                duration: props.duration / 1000,
                ease: 'easeOut',
                onUpdate: (latest: number) => {
                    displayValue.value = latest;
                    prevValue.value = latest;
                },
            });
        } catch {
            displayValue.value = numericValue;
            prevValue.value = numericValue;
        }
    },
    { immediate: true },
);

onUnmounted(() => {
    if (controls) {
        controls.stop();
    }
});

const shouldShowDecimals = computed(() => typeof props.value === 'string' && props.value.includes('.'));

const formattedValue = computed(() => {
    if (props.value === undefined || props.value === null) {
        return '-';
    }

    const decimalPlaces = shouldShowDecimals.value ? 2 : 0;
    return displayValue.value.toLocaleString('en-US', {
        minimumFractionDigits: decimalPlaces,
        maximumFractionDigits: decimalPlaces,
    });
});
</script>

<template>
    <span>{{ formattedValue }}</span>
</template>
