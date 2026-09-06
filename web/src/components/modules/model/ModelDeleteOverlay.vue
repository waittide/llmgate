<script setup lang="ts">
import { LuLoader, LuTrash2, LuX } from 'vue-icons-plus/lu';
import { motion } from 'motion-v';
import { useTranslations } from '@/composables/i18n';

defineProps<{
    layoutId: string;
    isPending?: boolean;
}>();

const emit = defineEmits<{
    (e: 'cancel'): void;
    (e: 'confirm'): void;
}>();

const { t } = useTranslations('model');
</script>

<template>
    <motion.div
        :layout-id="layoutId"
        class="absolute inset-0 flex items-center justify-center gap-3 bg-destructive p-4 rounded-2xl"
        :transition="{ type: 'spring', stiffness: 400, damping: 30 }"
    >
        <button
            type="button"
            @click="emit('cancel')"
            class="h-9 px-4 flex items-center justify-center gap-1.5 rounded-xl bg-destructive-foreground/20 text-destructive-foreground text-sm font-medium transition-all hover:bg-destructive-foreground/30 active:scale-[0.98]"
        >
            <LuX class="size-4" />
            {{ t('overlay.cancel') }}
        </button>
        <button
            type="button"
            @click="emit('confirm')"
            :disabled="isPending"
            class="h-9 px-4 flex items-center justify-center gap-1.5 rounded-xl bg-destructive-foreground text-destructive text-sm font-medium transition-all hover:bg-destructive-foreground/90 active:scale-[0.98] disabled:opacity-50 disabled:cursor-not-allowed"
        >
            <LuLoader v-if="isPending" class="size-4 animate-spin" />
            <LuTrash2 v-else class="size-4" />
            {{ isPending ? t('overlay.deleting') : t('overlay.confirmDelete') }}
        </button>
    </motion.div>
</template>
