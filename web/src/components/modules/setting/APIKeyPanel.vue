<script setup lang="ts">
import { computed, ref, useId } from 'vue';
import { LuKeyRound, LuLoader, LuPlus } from 'vue-icons-plus/lu';
import { motion, AnimatePresence } from 'motion-v';
import {
    useAPIKeyList,
    useCreateAPIKey,
    useUpdateAPIKey,
    useDeleteAPIKey,
    type APIKey,
} from '@/api/apikey';
import { useTranslations } from '@/composables/i18n';
import { toast } from '@/composables/toast';
import APIKeyForm, { type APIKeyFormData } from './APIKeyForm.vue';
import APIKeyStatsCard from './APIKeyStatsCard.vue';
import APIKeyKeyItem from './APIKeyKeyItem.vue';

const props = defineProps<{
    idPrefix: string;
    containerClassName: string;
    listClassName: string;
}>();

const { t } = useTranslations('setting');
const { data: apiKeys, isLoading: apiKeysLoading, error: apiKeysError } = useAPIKeyList();
const createAPIKey = useCreateAPIKey();
const updateAPIKey = useUpdateAPIKey();
const deleteAPIKey = useDeleteAPIKey();

const instanceId = useId();
const addLayoutId = `add-btn-${props.idPrefix}-${instanceId}`;
const statsPrefix = `${props.idPrefix}-stats-${instanceId}`;
const editPrefix = `${props.idPrefix}-edit-${instanceId}`;
const deletePrefix = `${props.idPrefix}-delete-`;

const isAdding = ref(false);
const viewingStats = ref<APIKey | null>(null);
const editingKey = ref<APIKey | null>(null);
const deletingId = ref<number | null>(null);

const sortedApiKeys = computed(() => {
    const list = apiKeys.value ?? [];
    return [...list].sort((a, b) => a.id - b.id);
});

const handleDelete = (id: number) => {
    deletingId.value = id;
    deleteAPIKey.mutate(id, {
        onSuccess: () => {
            toast.success(t('apiKey.toast.deleteSuccess'));
        },
        onError: (error) => {
            const msg = error instanceof Error ? error.message : undefined;
            toast.error(msg ? `${t('apiKey.toast.deleteError')}: ${msg}` : t('apiKey.toast.deleteError'));
        },
        onSettled: () => {
            if (deletingId.value === id) deletingId.value = null;
        },
    });
};

const closeAllOverlays = () => {
    isAdding.value = false;
    viewingStats.value = null;
    editingKey.value = null;
};

const disabledHeaderActions = computed(() => (
    createAPIKey.isPending.value || isAdding.value || !!viewingStats.value || !!editingKey.value
));

const handleCreate = (data: APIKeyFormData) => {
    createAPIKey.mutate(data, {
        onSuccess: () => {
            toast.success(t('apiKey.toast.createSuccess'));
            isAdding.value = false;
        },
        onError: (error) => {
            const msg = error instanceof Error ? error.message : undefined;
            toast.error(msg ? `${t('apiKey.toast.createError')}: ${msg}` : t('apiKey.toast.createError'));
        },
    });
};

const handleUpdate = (apiKey: APIKey, data: APIKeyFormData) => {
    updateAPIKey.mutate({ id: apiKey.id, ...data }, {
        onSuccess: () => {
            toast.success(t('apiKey.toast.updateSuccess'));
            editingKey.value = null;
        },
        onError: (error) => {
            const msg = error instanceof Error ? error.message : undefined;
            toast.error(msg ? `${t('apiKey.toast.updateError')}: ${msg}` : t('apiKey.toast.updateError'));
        },
    });
};
</script>

<template>
    <div :class="props.containerClassName">
        <div class="flex items-center justify-between gap-3">
            <h2 class="text-lg font-bold text-card-foreground flex items-center gap-2">
                <LuKeyRound class="h-5 w-5" />
                {{ t('apiKey.title') }}
            </h2>
            <div class="flex items-center gap-2">
                <motion.button
                    :layoutId="addLayoutId"
                    type="button"
                    @click="isAdding = true"
                    :disabled="disabledHeaderActions"
                    class="h-9 w-9 flex items-center justify-center rounded-lg bg-muted/60 text-muted-foreground transition-colors hover:bg-muted disabled:opacity-50"
                    :title="t('apiKey.add')"
                >
                    <LuPlus class="size-4" />
                </motion.button>
                <slot name="headerExtra" :disabled="disabledHeaderActions" />
            </div>
        </div>

        <!-- 添加表单浮层 -->
        <AnimatePresence>
            <motion.div
                v-if="isAdding"
                :layoutId="addLayoutId"
                class="absolute left-1/2 top-1/2 z-20 w-[min(420px,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 bg-card p-5 rounded-3xl border border-border max-h-[80vh] overflow-auto"
                :transition="{ type: 'spring', stiffness: 400, damping: 30 }"
            >
                <APIKeyForm
                    :is-pending="createAPIKey.isPending.value"
                    :submit-label="t('apiKey.form.create')"
                    @submit="handleCreate"
                    @close="isAdding = false"
                />
            </motion.div>
        </AnimatePresence>

        <!-- 查看统计浮层 -->
        <AnimatePresence>
            <APIKeyStatsCard
                v-if="viewingStats"
                :layout-id="`${statsPrefix}-${viewingStats.id}`"
                :api-key="viewingStats"
                @close="viewingStats = null"
            />
        </AnimatePresence>

        <!-- 编辑表单浮层 -->
        <AnimatePresence>
            <motion.div
                v-if="editingKey"
                :layoutId="`${editPrefix}-${editingKey.id}`"
                class="absolute left-1/2 top-1/2 z-20 w-[min(420px,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 bg-card p-5 rounded-3xl border border-border max-h-[80vh] overflow-auto"
                :transition="{ type: 'spring', stiffness: 400, damping: 30 }"
            >
                <APIKeyForm
                    :api-key="editingKey"
                    :is-pending="updateAPIKey.isPending.value"
                    :submit-label="t('apiKey.form.save')"
                    @submit="(data) => handleUpdate(editingKey!, data)"
                    @close="editingKey = null"
                />
            </motion.div>
        </AnimatePresence>

        <!-- 密钥列表 -->
        <div :class="props.listClassName">
            <div v-if="apiKeysLoading" class="h-full flex items-center justify-center text-sm text-muted-foreground">
                <LuLoader class="size-4 animate-spin" />
            </div>
            <div v-else-if="apiKeysError" class="h-full flex items-center justify-center text-sm text-destructive">
                {{ t('apiKey.loadFailed') }}
            </div>
            <div v-else-if="sortedApiKeys.length === 0" class="h-full flex items-center justify-center text-sm text-muted-foreground">
                {{ t('apiKey.empty') }}
            </div>
            <template v-else>
                <AnimatePresence>
                    <APIKeyKeyItem
                        v-for="apiKey in sortedApiKeys"
                        :key="apiKey.id"
                        :api-key="apiKey"
                        :stats-layout-id="`${statsPrefix}-${apiKey.id}`"
                        :edit-layout-id="`${editPrefix}-${apiKey.id}`"
                        :delete-layout-id="`${deletePrefix}${apiKey.id}`"
                        @view-stats="() => {
                            closeAllOverlays();
                            viewingStats = apiKey;
                        }"
                        @edit="() => {
                            closeAllOverlays();
                            editingKey = apiKey;
                        }"
                        @delete="() => handleDelete(apiKey.id)"
                        :is-deleting="deleteAPIKey.isPending.value && deletingId === apiKey.id"
                    />
                </AnimatePresence>
            </template>
        </div>
    </div>
</template>
