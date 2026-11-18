//! 창 사이 끌어다 놓기에 쓰는 내용물.

use std::path::PathBuf;
use unpack::Selection;

#[derive(Clone)]
pub enum DragItems {
    /// 폴더 창에서 끈 파일·폴더
    Files { pane: usize, paths: Vec<PathBuf> },
    /// 압축 파일 안에서 끈 항목 → 놓은 곳에 풀기
    Archive { pane: usize, archive: PathBuf, sel: Selection },
}

impl DragItems {
    pub fn pane(&self) -> usize {
        match self {
            DragItems::Files { pane, .. } | DragItems::Archive { pane, .. } => *pane,
        }
    }

    pub fn count(&self) -> usize {
        match self {
            DragItems::Files { paths, .. } => paths.len(),
            DragItems::Archive { sel, .. } => sel.names.len(),
        }
    }
}
