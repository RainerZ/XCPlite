/**********************************************************************************************************************
 *  COPYRIGHT
 *  -------------------------------------------------------------------------------------------------------------------
 *  \verbatim
 *  Copyright (c) 2026 by Vector Informatik GmbH. All rights reserved.
 *
 *                This software is copyright protected and proprietary to Vector Informatik GmbH.
 *                Vector Informatik GmbH grants to you only those rights as set out in the license conditions.
 *                All other rights remain with Vector Informatik GmbH.
 *  \endverbatim
 *  -------------------------------------------------------------------------------------------------------------------
 *  FILE DESCRIPTION
 *  -----------------------------------------------------------------------------------------------------------------*/
/*!        \file
 *        \brief  Generic RCU-style multi-page buffer for single-writer / multi-reader access.
 *
 *********************************************************************************************************************/

#ifndef LIB_MECA_INCLUDE_VES_MC_INTERNAL_RCU_H_
#define LIB_MECA_INCLUDE_VES_MC_INTERNAL_RCU_H_

/**********************************************************************************************************************
 *  INCLUDES
 *********************************************************************************************************************/
#include <atomic>
#include <cstddef>
#include <cstdint>
#include <limits>

#include "ves/array.h"

namespace ves {
namespace mc {
namespace internal {

/*!
 * \brief           Type identifying one page of an Rcu buffer
 * \details         Independent of the page count, so that reader-side types do not have to be parameterized with it.
 */
using RcuPageIndex = std::uint8_t;

/*!
 * \brief           Per-reader bookkeeping for one Rcu instance
 * \details         Private to the reading thread and never inspected by the writer. Holds the read-side nesting depth
 *                  and the page pinned on entering the outermost read-side critical section. Trivially destructible and
 *                  constant-initializable so that it can be held in thread storage without dynamic initialization or a
 *                  thread-exit destructor.
 */
struct RcuReaderState {
  /*! \brief          Read-side nesting depth of this reader */
  std::uint32_t depth{0U};
  /*! \brief          Page pinned while depth is non-zero */
  RcuPageIndex pinned_page_idx{0U};
};

/*!
 * \brief           Read-side interface of an RCU buffer
 * \vprivate
 * \tparam          T
 *                  Page payload type.
 */
template <typename T>
class RcuReadAccess {
 public:
  virtual ~RcuReadAccess() noexcept = default;

  /*!
   * \brief           Acquire pointer to the currently published page
   * \details         On entering the outermost read-side critical section the published page is pinned against
   *                  reclamation. Nested calls return the already pinned page.
   * \vprivate
   * \param[in,out]   reader
   *                  Bookkeeping of the calling reader. Must not be shared with another thread.
   * \return          Pointer to the pinned page. Never nullptr.
   * \context         Reader context.
   * \pre             The buffer has been initialized.
   * \post            Caller must invoke ReleaseReadPage() exactly once with the same \p reader.
   */
  virtual auto AcquireReadPage(RcuReaderState& reader) const noexcept -> T const* = 0;

  /*!
   * \brief           Release one previously acquired read-page reference
   * \details         Unpins the page when the outermost read-side critical section is left.
   * \vprivate
   * \param[in,out]   reader
   *                  Bookkeeping of the calling reader, as passed to the matching AcquireReadPage().
   * \context         Reader context.
   * \pre             Matching AcquireReadPage() has been called by the same reader.
   */
  virtual void ReleaseReadPage(RcuReaderState& reader) const noexcept = 0;
};

/*!
 * \brief           Lock-free multi-page RCU buffer for calibration payload exchange
 * \details         Provides a single-writer / multi-reader access scheme. One page is permanently owned by the writer
 *                  as its scratch page, one page is the currently published page, and the remaining pages are
 *                  reclamation candidates.
 *
 *                  Reclamation is driven by per-page reference counts rather than by reader progress:
 *                  - A reader pins the published page for the duration of its outermost read-side critical section.
 *                  - The writer publishes into any page that is neither its scratch page, nor the published page, nor
 *                    referenced by a reader.
 *
 *                  Publish is therefore independent of reader activity. It does not require a reader to hand a page
 *                  back, and it succeeds even when no reader ever locks this buffer.
 *
 *                  Visibility semantics:
 *                  - A successful TryPublish() is immediately visible to every read-side critical section entered
 *                    afterwards, on any thread, regardless of read locks held concurrently by other threads.
 *                  - Reads are monotonic: a read-side critical section never observes an older payload than one
 *                    observed by an earlier read-side critical section.
 *                  - A reader that is already inside a read-side critical section keeps observing the payload it pinned
 *                    on entry, so nested reads are coherent.
 *                  - Under contention, multiple writer-side updates may be coalesced in the scratch page before
 *                    becoming visible to readers.
 *
 *                  Usage limitations:
 *                  - Exactly one writer thread is supported.
 *                  - Any number of readers is supported.
 *                  - Each reader owns a ReaderState that must not be shared between threads or between Rcu instances.
 *                  - Each successful AcquireReadPage() call must be paired with ReleaseReadPage().
 *                  - InitializePages() must be called before first use and while no reader is active.
 *                  - Readers must treat returned pages as read-only.
 *                  - The reader fast path is lock-free but not wait-free: its validation loop retries when a publish
 *                    lands concurrently. Retries are bounded in practice by the publish rate.
 *                  - Publish fails while all reclamation candidates are pinned, which requires (kPageCount - 2)
 *                    concurrent read-side critical sections that each pinned a different page.
 *                  - A reader that never releases (for example a thread terminated inside its read-side critical
 *                    section) permanently retires one page.
 *                  - Not intended to provide fairness, bounded writer latency, or multi-writer semantics.
 *
 *                  Choosing PageCount:
 *                  - Three pages is the structural minimum and is sufficient to keep publish independent of reader
 *                    progress. With a single reclamation candidate, however, a reader holding a page blocks the second
 *                    of two consecutive publishes until it releases, and one reader that never releases blocks
 *                    publishing permanently.
 *                  - Each further page absorbs one more concurrently pinned page, and one more reader that never
 *                    releases. PageCount - 3 such readers are tolerated.
 *                  - Visibility latency does not depend on PageCount. Only the publish rate under contention does.
 * \tparam          T
 *                  Page payload type. Must be copy-assignable because publish copies full page content.
 * \tparam          PageCount
 *                  Number of pages: one writer scratch page, one published page and PageCount - 2 reclamation
 *                  candidates. Must be at least 3 and at most 255.
 */
template <typename T, std::size_t PageCount>
class Rcu final : public RcuReadAccess<T> {
 public:
  /*! \brief          Type identifying one page of this buffer */
  using PageIndex = RcuPageIndex;

  /*! \brief          Page count: one writer scratch page, one published page, and the reclamation candidates */
  static constexpr std::size_t kPageCount{PageCount};
  /*! \brief          Sentinel page index meaning that no page is currently available as a publish target */
  static constexpr PageIndex kInvalidPageIndex{static_cast<PageIndex>(kPageCount)};

  static_assert(kPageCount >= 3U,
                "An RCU buffer needs at least 3 pages: one writer scratch page, one published page and one "
                "publish target.");
  static_assert(kPageCount <= static_cast<std::size_t>(std::numeric_limits<PageIndex>::max()),
                "At most 255 pages are supported: a page index must fit into PageIndex, and kInvalidPageIndex "
                "occupies the value just above the highest valid index.");
  static_assert(std::atomic<PageIndex>::is_always_lock_free,
                "The published page index must be lock-free on this platform, otherwise the reader path would take "
                "a lock.");
  static_assert(std::atomic<std::uint32_t>::is_always_lock_free,
                "The page reference counts must be lock-free on this platform, otherwise the reader path would take "
                "a lock.");

  /*! \brief          Per-reader bookkeeping for one Rcu instance, see RcuReaderState */
  using ReaderState = RcuReaderState;

  /*!
   * \brief           Initialize all pages and reset runtime state
   * \param[in]       initial_page
   *                  Initial value copied into all RCU pages.
   * \context         Writer context before concurrent use.
   * \pre             Must be called before AcquireReadPage() / TryPublish(), and while no reader is active.
   */
  void InitializePages(T const& initial_page) noexcept {
    for (auto& page : pages_) {
      page = initial_page;
    }

    for (auto& page_ref : page_refs_) {
      page_ref.store(0U, std::memory_order_relaxed);
    }

    published_page_idx_.store(kFirstPublishedPageIdx, std::memory_order_seq_cst);
  }

  /*!
   * \brief           Acquire pointer to the currently published page
   * \details         On entering the outermost read-side critical section the published page is pinned against
   *                  reclamation. Nested calls return the already pinned page.
   * \param[in,out]   reader
   *                  Bookkeeping of the calling reader. Must not be shared with another thread.
   * \return          Pointer to the pinned page. Never nullptr.
   * \context         Reader context.
   * \pre             InitializePages() has been called.
   * \post            Caller must invoke ReleaseReadPage() exactly once with the same \p reader.
   */
  auto AcquireReadPage(ReaderState& reader) const noexcept -> T const* final {
    if (reader.depth == 0U) {
      bool pinned{false};
      while (!pinned) {
        PageIndex const page_idx{published_page_idx_.load(std::memory_order_seq_cst)};

        static_cast<void>(page_refs_[page_idx].fetch_add(1U, std::memory_order_seq_cst));

        // Still the published page? If so it holds complete data and the writer will not target it, because the
        // writer never publishes into the published page and always copies before it announces.
        if (published_page_idx_.load(std::memory_order_seq_cst) == page_idx) {
          reader.pinned_page_idx = page_idx;
          pinned = true;
        } else {
          static_cast<void>(page_refs_[page_idx].fetch_sub(1U, std::memory_order_seq_cst));
        }
      }
    }

    ++reader.depth;
    return &pages_[reader.pinned_page_idx];
  }

  /*!
   * \brief           Release one previously acquired read-page reference
   * \details         Unpins the page when the outermost read-side critical section is left.
   * \param[in,out]   reader
   *                  Bookkeeping of the calling reader, as passed to the matching AcquireReadPage().
   * \context         Reader context.
   * \pre             Matching AcquireReadPage() has been called by the same reader.
   */
  void ReleaseReadPage(ReaderState& reader) const noexcept final {
    if (reader.depth != 0U) {
      --reader.depth;
      if (reader.depth == 0U) {
        static_cast<void>(page_refs_[reader.pinned_page_idx].fetch_sub(1U, std::memory_order_seq_cst));
      }
    }
  }

  /*!
   * \brief           Attempt to publish writer data to readers
   * \details         Copies the scratch page into a reclamation candidate and announces it as the published page. Fails
   *                  only while every reclamation candidate is pinned by a reader.
   * \return          true if publish succeeded; false if no publish target is currently available.
   * \context         Single writer context only.
   * \pre             InitializePages() has been called.
   */
  auto TryPublish() noexcept -> bool {
    PageIndex const current_page_idx{published_page_idx_.load(std::memory_order_seq_cst)};
    PageIndex const target_page_idx{FindPublishTarget(current_page_idx)};

    bool published{false};
    if (target_page_idx != kInvalidPageIndex) {
      pages_[target_page_idx] = pages_[kWritePageIdx];
      // Announce only once the copy is complete: readers rely on this order.
      published_page_idx_.store(target_page_idx, std::memory_order_seq_cst);
      published = true;
    }

    return published;
  }

  /*!
   * \brief           Get mutable access to the writer-owned scratch page
   * \details         The scratch page always holds the most recent writer state, including changes not yet published.
   * \return          Pointer to writer scratch page.
   * \context         Single writer context only.
   * \pre             InitializePages() has been called.
   */
  auto GetWritePage() noexcept -> T* { return &pages_[kWritePageIdx]; }

  /*!
   * \brief           Get read-only access to the currently published page
   * \details         Intended for the writer to inspect what it has just published. The writer never publishes into the
   *                  published page, so the content is stable for as long as the writer does not publish again.
   * \return          Pointer to the published page.
   * \context         Single writer context only.
   * \pre             InitializePages() has been called.
   */
  auto GetPublishedPage() const noexcept -> T const* {
    return &pages_[published_page_idx_.load(std::memory_order_seq_cst)];
  }

 private:
  /*! \brief          Page permanently owned by the writer as its scratch page */
  static constexpr PageIndex kWritePageIdx{0U};
  /*! \brief          Page published by InitializePages() */
  static constexpr PageIndex kFirstPublishedPageIdx{1U};

  /*! \brief          Advance a page index, wrapping at kPageCount */
  static constexpr auto NextPageIndex(PageIndex page_idx) noexcept -> PageIndex {
    PageIndex const next_page_idx{static_cast<PageIndex>(page_idx + 1U)};
    return (static_cast<std::size_t>(next_page_idx) == kPageCount) ? PageIndex{0U} : next_page_idx;
  }

  /*!
   * \brief           Find a page that may be overwritten by the next publish
   * \details         Excludes the writer scratch page, the published page, and every page pinned by a reader.
   *
   *                  The search starts just above the published page and wraps, which makes it round-robin: the
   *                  published page is the one most recently written, so scanning upwards from it reaches every other
   *                  candidate before coming back to it. A retired page is therefore not reused until the other
   *                  candidates have been, which gives a reader that is still validating the widest possible window.
   * \param[in]       published_page_idx
   *                  Index of the currently published page.
   * \return          Index of a usable publish target, or kInvalidPageIndex if none is available.
   * \context         Single writer context only.
   */
  auto FindPublishTarget(PageIndex published_page_idx) const noexcept -> PageIndex {
    PageIndex result{kInvalidPageIndex};
    PageIndex candidate_page_idx{NextPageIndex(published_page_idx)};

    for (std::size_t attempt{0U}; attempt < kPageCount; ++attempt) {
      bool const is_reserved{(candidate_page_idx == kWritePageIdx) || (candidate_page_idx == published_page_idx)};
      if ((!is_reserved) && (page_refs_[candidate_page_idx].load(std::memory_order_seq_cst) == 0U)) {
        result = candidate_page_idx;
        break;
      }

      candidate_page_idx = NextPageIndex(candidate_page_idx);
    }

    return result;
  }

  ::ves::Array<T, kPageCount> pages_{};

  /*! \brief          Index of the currently published page */
  std::atomic<PageIndex> published_page_idx_{kFirstPublishedPageIdx};
  /*! \brief          Mutable because the const reader path pins and unpins pages */
  mutable ::ves::Array<std::atomic<std::uint32_t>, kPageCount> page_refs_{};
};

}  // namespace internal
}  // namespace mc
}  // namespace ves

#endif  // LIB_MECA_INCLUDE_VES_MC_INTERNAL_RCU_H_
