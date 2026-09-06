<script setup lang="ts">
import { ref } from 'vue';
import { LuInfo, LuPencil, LuTrash2, LuX } from 'vue-icons-plus/lu';
import { motion, AnimatePresence } from 'motion-v';
import type { APIKey } from '@/api/apikey';
import CopyIconButton from '@/components/common/CopyButton.vue';
import { useTranslations } from '@/composables/i18n';

const props = defineProps<{
    apiKey: APIKey;
    statsLayoutId: string;
    editLayoutId: string;
    deleteLayoutId: string;
    isDeleting: boolean;
}>();

const emit = defineEmits<{
    (e: 'viewStats'): void;
    (e: 'edit'): void;
    (e: 'delete'): void;
}>();

const { t } = useTranslations('setting');
const confirmDelete = ref(false);
</script>

<template>
    <motion.div
        layout
        :exit="{ opacity: 0, scale: 0.95, transition: { duration: 0.2 } }"
        :transition="{ type: 'spring', stiffness: 500, damping: 30 }"
        class="group relative flex items-center justify-between gap-3 p-3 rounded-xl bg-muted/50 overflow-hidden origin-top"
    >
        <span class="text-sm font-medium truncate">{{ props.apiKey.name }}</span>

        <div class="flex items-center gap-1.5">
            <motion.button
                type="button"
                :layoutId="props.statsLayoutId"
                @click="emit('viewStats')"
                class="flex size-8 items-center justify-center rounded-lg bg-muted/60 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground active:scale-95"
                title="Stats"
            >
                <LuInfo class="size-4" />
            </motion.button>
            <motion.button
                type="button"
                :layoutId="props.editLayoutId"
                @click="emit('edit')"
                class="flex size-8 items-center justify-center rounded-lg bg-muted/60 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground active:scale-95"
                title="Edit"
            >
                <LuPencil class="size-4" />
            </motion.button>
            <CopyIconButton
                :text="props.apiKey.api_key"
                class-name="flex size-8 items-center justify-center rounded-lg bg-primary/10 text-primary transition-all hover:bg-primary hover:text-primary-foreground active:scale-95"
                copy-icon-class-name="size-4"
                check-icon-class-name="size-4"
            />

            <motion.button
                v-if="!confirmDelete"
                :layoutId="props.deleteLayoutId"
                @click="confirmDelete = true"
                class="flex size-8 items-center justify-center rounded-lg bg-destructive/10 text-destructive transition-colors hover:bg-destructive hover:text-destructive-foreground"
            >
                <LuTrash2 class="size-4" />
            </motion.button>
        </div>

        <AnimatePresence>
            <motion.div
                v-if="confirmDelete"
                :layoutId="props.deleteLayoutId"
                class="absolute inset-0 flex items-center justify-center gap-2 bg-destructive p-3 rounded-xl"
                :transition="{ type: 'spring', stiffness: 400, damping: 30 }"
            >
                <button
                    type="button"
                    @click="confirmDelete = false"
                    class="flex size-8 items-center justify-center rounded-lg bg-destructive-foreground/20 text-destructive-foreground transition-all hover:bg-destructive-foreground/30 active:scale-95"
                >
                    <LuX class="size-4" />
                </button>
                <button
                    type="button"
                    @click="emit('delete')"
                    :disabled="props.isDeleting"
                    class="flex-1 h-8 flex items-center justify-center gap-1.5 rounded-lg bg-destructive-foreground text-destructive text-sm font-medium transition-all hover:bg-destructive-foreground/90 active:scale-[0.98] disabled:opacity-50"
                >
                    <LuTrash2 class="size-3.5" />
                    {{ props.isDeleting ? '...' : t('apiKey.form.confirm') }}
                </button>
            </motion.div>
        </AnimatePresence>
    </motion.div>
</template>
