// Copyright 2020 The Jujutsu Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::collections::HashSet;

use itertools::Itertools as _;
use thiserror::Error;

use crate::backend::CommitId;
use crate::op_store;
use crate::op_store::BookmarkTarget;
use crate::op_store::RefTarget;
use crate::op_store::RefTargetOptionExt as _;
use crate::op_store::RemoteRef;
use crate::op_store::WorkspaceId;
use crate::ref_name::RefName;
use crate::ref_name::RefNameBuf;
use crate::ref_name::RemoteName;
use crate::ref_name::RemoteRefSymbol;
use crate::refs;
use crate::refs::LocalAndRemoteRef;
use crate::str_util::StringPattern;

/// A wrapper around [`op_store::View`] that defines additional methods.
#[derive(PartialEq, Eq, Debug, Clone)]
pub struct View {
    data: op_store::View,
}

impl View {
    pub fn new(op_store_view: op_store::View) -> Self {
        View {
            data: op_store_view,
        }
    }

    pub fn wc_commit_ids(&self) -> &BTreeMap<WorkspaceId, CommitId> {
        &self.data.wc_commit_ids
    }

    pub fn get_wc_commit_id(&self, workspace_id: &WorkspaceId) -> Option<&CommitId> {
        self.data.wc_commit_ids.get(workspace_id)
    }

    pub fn workspaces_for_wc_commit_id(&self, commit_id: &CommitId) -> Vec<WorkspaceId> {
        let mut workspaces_ids = vec![];
        for (workspace_id, wc_commit_id) in &self.data.wc_commit_ids {
            if wc_commit_id == commit_id {
                workspaces_ids.push(workspace_id.clone());
            }
        }
        workspaces_ids
    }

    pub fn is_wc_commit_id(&self, commit_id: &CommitId) -> bool {
        self.data.wc_commit_ids.values().contains(commit_id)
    }

    pub fn heads(&self) -> &HashSet<CommitId> {
        &self.data.head_ids
    }

    /// Iterates pair of local and remote bookmarks by bookmark name.
    pub fn bookmarks(&self) -> impl Iterator<Item = (&RefName, BookmarkTarget<'_>)> {
        op_store::merge_join_bookmark_views(&self.data.local_bookmarks, &self.data.remote_views)
    }

    pub fn tags(&self) -> &BTreeMap<RefNameBuf, RefTarget> {
        &self.data.tags
    }

    pub fn git_refs(&self) -> &BTreeMap<String, RefTarget> {
        &self.data.git_refs
    }

    pub fn git_head(&self) -> &RefTarget {
        &self.data.git_head
    }

    pub fn set_wc_commit(&mut self, workspace_id: WorkspaceId, commit_id: CommitId) {
        self.data.wc_commit_ids.insert(workspace_id, commit_id);
    }

    pub fn remove_wc_commit(&mut self, workspace_id: &WorkspaceId) {
        self.data.wc_commit_ids.remove(workspace_id);
    }

    pub fn rename_workspace(
        &mut self,
        old_workspace_id: &WorkspaceId,
        new_workspace_id: WorkspaceId,
    ) -> Result<(), RenameWorkspaceError> {
        if self.data.wc_commit_ids.contains_key(&new_workspace_id) {
            return Err(RenameWorkspaceError::WorkspaceAlreadyExists {
                workspace_id: new_workspace_id.as_str().to_owned(),
            });
        }
        let wc_commit_id = self
            .data
            .wc_commit_ids
            .remove(old_workspace_id)
            .ok_or_else(|| RenameWorkspaceError::WorkspaceDoesNotExist {
                workspace_id: old_workspace_id.as_str().to_owned(),
            })?;
        self.data
            .wc_commit_ids
            .insert(new_workspace_id, wc_commit_id);
        Ok(())
    }

    pub fn add_head(&mut self, head_id: &CommitId) {
        self.data.head_ids.insert(head_id.clone());
    }

    pub fn remove_head(&mut self, head_id: &CommitId) {
        self.data.head_ids.remove(head_id);
    }

    /// Iterates local bookmark `(name, target)`s in lexicographical order.
    pub fn local_bookmarks(&self) -> impl Iterator<Item = (&RefName, &RefTarget)> {
        self.data
            .local_bookmarks
            .iter()
            .map(|(name, target)| (name.as_ref(), target))
    }

    /// Iterates local bookmarks `(name, target)` in lexicographical order where
    /// the target adds `commit_id`.
    pub fn local_bookmarks_for_commit<'a, 'b>(
        &'a self,
        commit_id: &'b CommitId,
    ) -> impl Iterator<Item = (&'a RefName, &'a RefTarget)> + use<'a, 'b> {
        self.local_bookmarks()
            .filter(|(_, target)| target.added_ids().contains(commit_id))
    }

    /// Iterates local bookmark `(name, target)`s matching the given pattern.
    /// Entries are sorted by `name`.
    pub fn local_bookmarks_matching<'a, 'b>(
        &'a self,
        pattern: &'b StringPattern,
    ) -> impl Iterator<Item = (&'a RefName, &'a RefTarget)> + use<'a, 'b> {
        pattern
            .filter_btree_map_as_deref(&self.data.local_bookmarks)
            .map(|(name, target)| (name.as_ref(), target))
    }

    pub fn get_local_bookmark(&self, name: &RefName) -> &RefTarget {
        self.data.local_bookmarks.get(name).flatten()
    }

    /// Sets local bookmark to point to the given target. If the target is
    /// absent, and if no associated remote bookmarks exist, the bookmark
    /// will be removed.
    pub fn set_local_bookmark_target(&mut self, name: &RefName, target: RefTarget) {
        if target.is_present() {
            self.data.local_bookmarks.insert(name.to_owned(), target);
        } else {
            self.data.local_bookmarks.remove(name);
        }
    }

    /// Iterates over `(symbol, remote_ref)` for all remote bookmarks in
    /// lexicographical order.
    pub fn all_remote_bookmarks(&self) -> impl Iterator<Item = (RemoteRefSymbol<'_>, &RemoteRef)> {
        op_store::flatten_remote_bookmarks(&self.data.remote_views)
    }

    /// Iterates over `(name, remote_ref)`s for all remote bookmarks of the
    /// specified remote in lexicographical order.
    pub fn remote_bookmarks(
        &self,
        remote_name: &RemoteName,
    ) -> impl Iterator<Item = (&RefName, &RemoteRef)> + use<'_> {
        let maybe_remote_view = self.data.remote_views.get(remote_name);
        maybe_remote_view
            .map(|remote_view| {
                remote_view
                    .bookmarks
                    .iter()
                    .map(|(name, remote_ref)| (name.as_ref(), remote_ref))
            })
            .into_iter()
            .flatten()
    }

    /// Iterates over `(symbol, remote_ref)`s for all remote bookmarks of the
    /// specified remote that match the given pattern.
    ///
    /// Entries are sorted by `symbol`, which is `(name, remote)`.
    pub fn remote_bookmarks_matching<'a, 'b>(
        &'a self,
        bookmark_pattern: &'b StringPattern,
        remote_pattern: &'b StringPattern,
    ) -> impl Iterator<Item = (RemoteRefSymbol<'a>, &'a RemoteRef)> + use<'a, 'b> {
        // Use kmerge instead of flat_map for consistency with all_remote_bookmarks().
        remote_pattern
            .filter_btree_map_as_deref(&self.data.remote_views)
            .map(|(remote, remote_view)| {
                bookmark_pattern
                    .filter_btree_map_as_deref(&remote_view.bookmarks)
                    .map(|(name, remote_ref)| (name.to_remote_symbol(remote), remote_ref))
            })
            .kmerge_by(|(symbol1, _), (symbol2, _)| symbol1 < symbol2)
    }

    pub fn get_remote_bookmark(&self, symbol: RemoteRefSymbol<'_>) -> &RemoteRef {
        if let Some(remote_view) = self.data.remote_views.get(symbol.remote) {
            remote_view.bookmarks.get(symbol.name).flatten()
        } else {
            RemoteRef::absent_ref()
        }
    }

    /// Sets remote-tracking bookmark to the given target and state. If the
    /// target is absent, the bookmark will be removed.
    pub fn set_remote_bookmark(&mut self, symbol: RemoteRefSymbol<'_>, remote_ref: RemoteRef) {
        if remote_ref.is_present() {
            let remote_view = self
                .data
                .remote_views
                .entry(symbol.remote.to_owned())
                .or_default();
            remote_view
                .bookmarks
                .insert(symbol.name.to_owned(), remote_ref);
        } else if let Some(remote_view) = self.data.remote_views.get_mut(symbol.remote) {
            remote_view.bookmarks.remove(symbol.name);
        }
    }

    /// Iterates over `(name, {local_ref, remote_ref})`s for every bookmark
    /// present locally and/or on the specified remote, in lexicographical
    /// order.
    ///
    /// Note that this does *not* take into account whether the local bookmark
    /// tracks the remote bookmark or not. Missing values are represented as
    /// RefTarget::absent_ref() or RemoteRef::absent_ref().
    pub fn local_remote_bookmarks(
        &self,
        remote_name: &RemoteName,
    ) -> impl Iterator<Item = (&RefName, LocalAndRemoteRef<'_>)> + use<'_> {
        refs::iter_named_local_remote_refs(
            self.local_bookmarks(),
            self.remote_bookmarks(remote_name),
        )
        .map(|(name, (local_target, remote_ref))| {
            let targets = LocalAndRemoteRef {
                local_target,
                remote_ref,
            };
            (name, targets)
        })
    }

    /// Iterates over `(name, TrackingRefPair {local_ref, remote_ref})`s for
    /// every bookmark with a name that matches the given pattern, and that is
    /// present locally and/or on the specified remote.
    ///
    /// Entries are sorted by `name`.
    ///
    /// Note that this does *not* take into account whether the local bookmark
    /// tracks the remote bookmark or not. Missing values are represented as
    /// RefTarget::absent_ref() or RemoteRef::absent_ref().
    pub fn local_remote_bookmarks_matching<'a, 'b>(
        &'a self,
        bookmark_pattern: &'b StringPattern,
        remote_name: &RemoteName,
    ) -> impl Iterator<Item = (&'a RefName, LocalAndRemoteRef<'a>)> + use<'a, 'b> {
        // Change remote_name to StringPattern if needed, but merge-join adapter won't
        // be usable.
        let maybe_remote_view = self.data.remote_views.get(remote_name);
        refs::iter_named_local_remote_refs(
            bookmark_pattern.filter_btree_map_as_deref(&self.data.local_bookmarks),
            maybe_remote_view
                .map(|remote_view| {
                    bookmark_pattern.filter_btree_map_as_deref(&remote_view.bookmarks)
                })
                .into_iter()
                .flatten(),
        )
        .map(|(name, (local_target, remote_ref))| {
            let targets = LocalAndRemoteRef {
                local_target,
                remote_ref,
            };
            (name.as_ref(), targets)
        })
    }

    pub fn remove_remote(&mut self, remote_name: &RemoteName) {
        self.data.remote_views.remove(remote_name);
    }

    pub fn rename_remote(&mut self, old: &RemoteName, new: &RemoteName) {
        if let Some(remote_view) = self.data.remote_views.remove(old) {
            self.data.remote_views.insert(new.to_owned(), remote_view);
        }
    }

    pub fn get_tag(&self, name: &RefName) -> &RefTarget {
        self.data.tags.get(name).flatten()
    }

    /// Iterates tags `(name, target)`s matching the given pattern. Entries
    /// are sorted by `name`.
    pub fn tags_matching<'a, 'b>(
        &'a self,
        pattern: &'b StringPattern,
    ) -> impl Iterator<Item = (&'a RefName, &'a RefTarget)> + use<'a, 'b> {
        pattern
            .filter_btree_map_as_deref(&self.data.tags)
            .map(|(name, target)| (name.as_ref(), target))
    }

    /// Sets tag to point to the given target. If the target is absent, the tag
    /// will be removed.
    pub fn set_tag_target(&mut self, name: &RefName, target: RefTarget) {
        if target.is_present() {
            self.data.tags.insert(name.to_owned(), target);
        } else {
            self.data.tags.remove(name);
        }
    }

    pub fn get_git_ref(&self, name: &str) -> &RefTarget {
        self.data.git_refs.get(name).flatten()
    }

    /// Sets the last imported Git ref to point to the given target. If the
    /// target is absent, the reference will be removed.
    pub fn set_git_ref_target(&mut self, name: &str, target: RefTarget) {
        if target.is_present() {
            self.data.git_refs.insert(name.to_owned(), target);
        } else {
            self.data.git_refs.remove(name);
        }
    }

    /// Sets Git HEAD to point to the given target. If the target is absent, the
    /// reference will be cleared.
    pub fn set_git_head_target(&mut self, target: RefTarget) {
        self.data.git_head = target;
    }

    /// Iterates all commit ids referenced by this view.
    ///
    /// This can include hidden commits referenced by remote bookmarks, previous
    /// positions of conflicted bookmarks, etc. The ancestors and predecessors
    /// of the returned commits should be considered reachable from the
    /// view. Use this to build commit index from scratch.
    ///
    /// The iteration order is unspecified, and may include duplicated entries.
    pub fn all_referenced_commit_ids(&self) -> impl Iterator<Item = &CommitId> {
        // Include both added/removed ids since ancestry information of old
        // references will be needed while merging views.
        fn ref_target_ids(target: &RefTarget) -> impl Iterator<Item = &CommitId> {
            target.as_merge().iter().flatten()
        }

        // Some of the fields (e.g. wc_commit_ids) would be redundant, but let's
        // not be smart here. Callers will build a larger set of commits anyway.
        let op_store::View {
            head_ids,
            local_bookmarks,
            tags,
            remote_views,
            git_refs,
            git_head,
            wc_commit_ids,
        } = &self.data;
        itertools::chain!(
            head_ids,
            local_bookmarks.values().flat_map(ref_target_ids),
            tags.values().flat_map(ref_target_ids),
            remote_views.values().flat_map(|remote_view| {
                let op_store::RemoteView { bookmarks } = remote_view;
                bookmarks
                    .values()
                    .flat_map(|remote_ref| ref_target_ids(&remote_ref.target))
            }),
            git_refs.values().flat_map(ref_target_ids),
            ref_target_ids(git_head),
            wc_commit_ids.values()
        )
    }

    pub fn set_view(&mut self, data: op_store::View) {
        self.data = data;
    }

    pub fn store_view(&self) -> &op_store::View {
        &self.data
    }

    pub fn store_view_mut(&mut self) -> &mut op_store::View {
        &mut self.data
    }
}

/// Error from attempts to rename a workspace
#[derive(Debug, Error)]
pub enum RenameWorkspaceError {
    #[error("Workspace {workspace_id} not found")]
    WorkspaceDoesNotExist { workspace_id: String },

    #[error("Workspace {workspace_id} already exists")]
    WorkspaceAlreadyExists { workspace_id: String },
}
