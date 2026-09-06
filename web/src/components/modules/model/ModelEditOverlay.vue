<script setup lang="ts">
import { ref, onMounted, nextTick } from 'vue';
import { LuCheck, LuLoader, LuX } from 'vue-icons-plus/lu';
import { motion } from 'motion-v';
import { useTranslations } from '@/composables/i18n';
import { Input } from '@/components/ui/input';

export type EditValues = {
    input: string;
    output: string;
    cache_read: string;
    cache_write: string;
};

defineProps<{
    layoutId: string;
    modelName: string;
    brandColor: string;
    editValues: EditValues;
    isPending?: boolean;
}>();

const emit = defineEmits<{
    (e: 'cancel'): void;
    (e: 'save'): void;
    (e: 'heightChange', height: number): void;
}>();

const { t } = useTranslations('model');
const containerRef = ref<HTMLElement | null>(null);

const setRef = (el: unknown) => {
    const target = el as { $el?: unknown } | HTMLElement | null;
    const domEl =
        target && typeof target === 'object' && '$el' in target && target.$el instanceof HTMLElement
            ? target.$el
            : target instanceof HTMLElement
              ? target
              : null;
    containerRef.value = domEl;
    if (domEl) {
        emit('heightChange', domEl.offsetHeight);
    }
};

onMounted(async () => {
    await nextTick();
    if (containerRef.value) {
        emit('heightChange', containerRef.value.offsetHeight);
    }
});
</script>

<template>
    <motion.div
        :ref="setRef"
        :layout-id="layoutId"
        class="flex flex-col bg-card p-5 rounded-3xl border border-border"
        :transition="{ type: 'spring', stiffness: 400, damping: 30 }"
    >
        <h3 class="text-sm font-semibold text-card-foreground line-clamp-1 mb-3">
            {{ modelName }}
        </h3>

        <div class="grid grid-cols-2 gap-2">
            <label class="grid gap-1 text-xs text-muted-foreground">
                {{ t('overlay.input') }}
                <Input
                    type="number"
                    step="any"
                    v-model="editValues.input"
                    class="h-9 text-sm rounded-xl"
                />
            </label>
            <label class="grid gap-1 text-xs text-muted-foreground">
                {{ t('overlay.output') }}
                <Input
                    type="number"
                    step="any"
                    v-model="editValues.output"
                    class="h-9 text-sm rounded-xl"
                />
            </label>
            <label class="grid gap-1 text-xs text-muted-foreground">
                {{ t('overlay.cacheRead') }}
                <Input
                    type="number"
                    step="any"
                    v-model="editValues.cache_read"
                    class="h-9 text-sm rounded-xl"
                />
            </label>
            <label class="grid gap-1 text-xs text-muted-foreground">
                {{ t('overlay.cacheWrite') }}
                <Input
                    type="number"
                    step="any"
                    v-model="editValues.cache_write"
                    class="h-9 text-sm rounded-xl"
                />
            </label>
        </div>

        <div class="flex gap-2 pt-2 mt-3">
            <button
                type="button"
                @click="emit('cancel')"
                :disabled="isPending"
                class="flex-1 h-9 flex items-center justify-center gap-1.5 rounded-xl bg-muted text-muted-foreground text-sm font-medium transition-all hover:bg-muted/80 active:scale-[0.98] disabled:opacity-50"
            >
                <LuX class="size-4" />
                {{ t('overlay.cancel') }}
            </button>
            <button
                type="button"
                @click="emit('save')"
                :disabled="isPending"
                class="flex-1 h-9 flex items-center justify-center gap-1.5 rounded-xl text-sm font-medium transition-all active:scale-[0.98] disabled:opacity-50"
                :style="{ backgroundColor: brandColor, color: '#fff' }"
            >
                <LuLoader v-if="isPending" class="size-4 animate-spin" />
                <LuCheck v-else class="size-4" />
                {{ t('overlay.save') }}
            </button>
        </div>
    </motion.div>
</template>
