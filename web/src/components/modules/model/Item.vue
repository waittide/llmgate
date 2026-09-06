<script setup lang="ts">
import { ref, computed, watch, onUnmounted, useId, useTemplateRef } from 'vue';
import { LuArrowDownToLine, LuArrowUpFromLine, LuPencil, LuTrash2 } from 'vue-icons-plus/lu';
import { motion, AnimatePresence } from 'motion-v';
import { useTranslations } from '@/composables/i18n';
import { toast } from '@/composables/toast';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { useUpdateModel, useDeleteModel, type ModelInfo } from '@/api/model';
import { getModelIcon } from './model-icons';
import { ModelDeleteOverlay, ModelEditOverlay, type EditValues } from './ItemOverlays';
import { cn } from '@/lib/utils';

export interface ModelItemProps {
    model: ModelInfo;
    layout?: 'grid' | 'list';
}

const props = withDefaults(defineProps<ModelItemProps>(), {
    layout: 'grid',
});

// 缓存编辑弹层实测高度，首次打开即可正确判断是否需要向上弹出
let cachedEditOverlayHeight = 0;

const { t } = useTranslations('model');
const isListLayout = computed(() => props.layout === 'list');
const isEditOpen = ref(false);
const confirmDelete = ref(false);
const overlayRect = ref<{ top: number; left: number; width: number } | null>(null);

const instanceId = useId();
const editLayoutId = computed(() => `edit-btn-${props.model.name}-${instanceId}`);
const deleteLayoutId = computed(() => `delete-btn-${props.model.name}-${instanceId}`);

const cardRef = useTemplateRef<HTMLElement>('cardRef');
const editButtonRef = useTemplateRef<HTMLButtonElement>('editButtonRef');
const editOverlayRef = useTemplateRef<HTMLDivElement>('editOverlayRef');

const editValues = ref<EditValues>({
    input: props.model.input.toString(),
    output: props.model.output.toString(),
    cache_read: props.model.cache_read.toString(),
    cache_write: props.model.cache_write.toString(),
});

const updateModel = useUpdateModel();
const deleteModel = useDeleteModel();

const iconConfig = computed(() => getModelIcon(props.model.name));

const updateOverlayRect = () => {
    const card = cardRef.value;
    if (!card) return;
    const rect = card.getBoundingClientRect();
    const height = cachedEditOverlayHeight;
    // 下方空间不足时改为向上弹出，避免最后一行弹层被视口底部截断
    const flipUp = height > 0 && rect.top + height > window.innerHeight;
    const top = flipUp
        ? Math.min(Math.max(rect.bottom - height, 0), Math.max(window.innerHeight - height, 0))
        : rect.top;

    if (
        overlayRect.value &&
        overlayRect.value.top === top &&
        overlayRect.value.left === rect.left &&
        overlayRect.value.width === rect.width
    ) {
        return;
    }
    overlayRect.value = { top, left: rect.left, width: rect.width };
};

const closeEdit = () => {
    isEditOpen.value = false;
};

const handleOverlayHeightChange = (height: number) => {
    if (height === cachedEditOverlayHeight) return;
    cachedEditOverlayHeight = height;
    updateOverlayRect();
};

const handleEditClick = () => {
    confirmDelete.value = false;
    editValues.value = {
        input: props.model.input.toString(),
        output: props.model.output.toString(),
        cache_read: props.model.cache_read.toString(),
        cache_write: props.model.cache_write.toString(),
    };
    // Ensure first open already has anchor geometry so layout animation can run.
    updateOverlayRect();
    isEditOpen.value = true;
};

const handleCancelEdit = () => {
    closeEdit();
};

const handleSaveEdit = () => {
    updateModel.mutate(
        {
            name: props.model.name,
            input: parseFloat(editValues.value.input) || 0,
            output: parseFloat(editValues.value.output) || 0,
            cache_read: parseFloat(editValues.value.cache_read) || 0,
            cache_write: parseFloat(editValues.value.cache_write) || 0,
        },
        {
            onSuccess: () => {
                closeEdit();
                toast.success(t('toast.updated'));
            },
            onError: (error) => {
                const message = error instanceof Error ? error.message : String(error);
                toast.error(message || t('toast.updateFailed'));
            },
        }
    );
};

const handleDeleteClick = () => {
    closeEdit();
    confirmDelete.value = true;
};

const handleCancelDelete = () => {
    confirmDelete.value = false;
};

const handleConfirmDelete = () => {
    deleteModel.mutate(props.model.name, {
        onSuccess: () => {
            confirmDelete.value = false;
            toast.success(t('toast.deleted'));
        },
        onError: (error) => {
            confirmDelete.value = false;
            const message = error instanceof Error ? error.message : String(error);
            toast.error(message || t('toast.deleteFailed'));
        },
    });
};

const handlePointerDown = (event: PointerEvent) => {
    const target = event.target as Node | null;
    if (!target) return;
    if (editOverlayRef.value?.contains(target)) return;
    if (editButtonRef.value?.contains(target)) return;
    closeEdit();
};

const handleKeyDown = (event: KeyboardEvent) => {
    if (event.key === 'Escape') closeEdit();
};

const cleanupListeners = () => {
    if (typeof window === 'undefined') return;
    window.removeEventListener('resize', updateOverlayRect);
    window.removeEventListener('scroll', updateOverlayRect, true);
    document.removeEventListener('pointerdown', handlePointerDown);
    document.removeEventListener('keydown', handleKeyDown);
};

watch(
    isEditOpen,
    (open) => {
        if (typeof window === 'undefined') return;
        if (open) {
            updateOverlayRect();
            window.addEventListener('resize', updateOverlayRect);
            window.addEventListener('scroll', updateOverlayRect, true);
            document.addEventListener('pointerdown', handlePointerDown);
            document.addEventListener('keydown', handleKeyDown);
        } else {
            cleanupListeners();
        }
    },
    { immediate: true }
);

onUnmounted(() => {
    cleanupListeners();
});
</script>

<template>
    <article
        ref="cardRef"
        :class="
            cn(
                'group relative rounded-3xl border border-border bg-card flex items-center gap-3 p-4',
                (isEditOpen || confirmDelete) && 'z-50'
            )
        "
    >
        <component
            :is="iconConfig.Icon"
            aria-hidden="true"
            :class="iconConfig.className"
            :size="52"
            :width="52"
            :height="52"
        />

        <div class="flex-1 min-w-0 flex flex-col justify-center gap-2">
            <Tooltip>
                <TooltipTrigger as-child>
                    <span class="w-fit max-w-full text-base font-semibold text-card-foreground leading-tight truncate">
                        {{ model.name }}
                    </span>
                </TooltipTrigger>
                <TooltipContent :key="model.name" side="top" :side-offset="10" align="center">
                    {{ model.name }}
                </TooltipContent>
            </Tooltip>

            <p
                v-if="isListLayout"
                class="flex items-center gap-2 overflow-hidden text-sm text-muted-foreground whitespace-nowrap"
            >
                <span class="inline-flex items-center gap-1">
                    <LuArrowDownToLine class="size-3.5 shrink-0" :style="{ color: iconConfig.color }" />
                    {{ t('card.inputCache') }}
                    <span class="tabular-nums">{{ model.input.toFixed(2) }}/{{ model.cache_read.toFixed(2) }}$</span>
                </span>
                <span class="text-muted-foreground/60">|</span>
                <span class="inline-flex items-center gap-1 overflow-hidden">
                    <LuArrowUpFromLine class="size-3.5 shrink-0" :style="{ color: iconConfig.color }" />
                    {{ t('card.outputCache') }}
                    <span class="tabular-nums truncate">{{ model.output.toFixed(2) }}/{{ model.cache_write.toFixed(2) }}$</span>
                </span>
            </p>
            <template v-else>
                <p class="flex items-center gap-1.5 text-sm text-muted-foreground">
                    <LuArrowDownToLine class="size-3.5" :style="{ color: iconConfig.color }" />
                    {{ t('card.inputCache') }}
                    <span class="tabular-nums">{{ model.input.toFixed(2) }}/{{ model.cache_read.toFixed(2) }}$</span>
                </p>

                <p class="flex items-center gap-1.5 text-sm text-muted-foreground">
                    <LuArrowUpFromLine class="size-3.5" :style="{ color: iconConfig.color }" />
                    {{ t('card.outputCache') }}
                    <span class="tabular-nums">{{ model.output.toFixed(2) }}/{{ model.cache_write.toFixed(2) }}$</span>
                </p>
            </template>
        </div>

        <div
            :class="
                cn(
                    isListLayout
                        ? 'shrink-0 flex items-center gap-2 self-center'
                        : 'shrink-0 flex flex-col justify-between self-stretch',
                    (isEditOpen || confirmDelete) && 'invisible pointer-events-none'
                )
            "
        >
            <motion.button
                ref="editButtonRef"
                :layout-id="editLayoutId"
                type="button"
                @click="handleEditClick"
                :disabled="isEditOpen || confirmDelete"
                class="h-9 w-9 flex items-center justify-center rounded-lg bg-muted/60 text-muted-foreground transition-colors hover:bg-muted disabled:opacity-50"
                :title="t('card.edit')"
            >
                <LuPencil class="size-4" />
            </motion.button>

            <motion.button
                :layout-id="deleteLayoutId"
                type="button"
                @click="handleDeleteClick"
                :disabled="isEditOpen || confirmDelete"
                class="h-9 w-9 flex items-center justify-center rounded-lg bg-destructive/10 text-destructive transition-colors hover:bg-destructive hover:text-destructive-foreground disabled:opacity-50"
                :title="t('card.delete')"
            >
                <LuTrash2 class="size-4" />
            </motion.button>
        </div>

        <AnimatePresence>
            <ModelDeleteOverlay
                v-if="confirmDelete"
                :layout-id="deleteLayoutId"
                :is-pending="deleteModel.isPending.value"
                @cancel="handleCancelDelete"
                @confirm="handleConfirmDelete"
            />
        </AnimatePresence>

        <Teleport to="body">
            <AnimatePresence @exit-complete="overlayRect = null">
                <div
                    v-if="isEditOpen && overlayRect"
                    ref="editOverlayRef"
                    class="fixed z-[90]"
                    :style="{
                        top: `${overlayRect.top}px`,
                        left: `${overlayRect.left}px`,
                        width: `${overlayRect.width}px`,
                    }"
                >
                    <ModelEditOverlay
                        :layout-id="editLayoutId"
                        :model-name="model.name"
                        :brand-color="iconConfig.color"
                        :edit-values="editValues"
                        :is-pending="updateModel.isPending.value"
                        @height-change="handleOverlayHeightChange"
                        @cancel="handleCancelEdit"
                        @save="handleSaveEdit"
                    />
                </div>
            </AnimatePresence>
        </Teleport>
    </article>
</template>
