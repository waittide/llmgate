<script setup lang="ts">
import type { GroupItem } from '@/api/group';
import {
    MorphingDialogClose,
    MorphingDialogDescription,
    MorphingDialogTitle,
    useMorphingDialog,
} from '@/components/ui/morphing-dialog';
import { useCreateGroup } from '@/api/group';
import { useTranslations } from '@/composables/i18n';
import { toast } from '@/composables/toast';
import GroupEditor, { type GroupEditorValues } from './Editor.vue';

const { setIsOpen } = useMorphingDialog();
const createGroup = useCreateGroup();
const { t } = useTranslations('group');

function handleSubmit({ name, mode, relay_config, members }: GroupEditorValues) {
    const items: GroupItem[] = members.map((member, index) => ({
        channel_model_id: member.channel_model_id,
        priority: index + 1,
    }));

    createGroup.mutate(
        { name, mode, active_item_id: 0, relay_config, items },
        {
            onSuccess: () => setIsOpen(false),
            onError: (error: Error) => {
                toast.error(t('toast.createFailed') + (error.message ? `: ${error.message}` : ''));
            },
        }
    );
}
</script>

<template>
    <div class="w-screen max-w-full md:max-w-4xl h-[calc(100vh-2rem)] min-h-0 flex flex-col">
        <MorphingDialogTitle class="shrink-0">
            <header class="mb-5 flex items-center justify-between">
                <h2 class="text-2xl font-bold text-card-foreground">
                    {{ t('create.title') }}
                </h2>
                <MorphingDialogClose
                    class="relative right-0 top-0"
                    :variants="{
                        initial: { opacity: 0, scale: 0.8 },
                        animate: { opacity: 1, scale: 1 },
                        exit: { opacity: 0, scale: 0.8 },
                    }"
                />
            </header>
        </MorphingDialogTitle>
        <MorphingDialogDescription class="flex-1 min-h-0 overflow-hidden">
            <GroupEditor
                :submit-text="t('create.submit')"
                :submitting-text="t('create.submitting')"
                :is-submitting="createGroup.isPending.value"
                @submit="handleSubmit"
            />
        </MorphingDialogDescription>
    </div>
</template>
