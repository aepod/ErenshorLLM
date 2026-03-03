using System;
using System.Collections.Generic;
using UnityEngine;

namespace ErenshorLLMDialog.Pipeline
{
    /// <summary>
    /// Priority levels for paraphrase requests.
    /// Higher value = higher priority = processed first.
    /// </summary>
    public enum ParaphrasePriority
    {
        /// <summary>Skip paraphrase entirely -- pass original text through.</summary>
        Skip = 0,
        /// <summary>Background ambient: random shout/say from non-nearby sims.</summary>
        Low = 1,
        /// <summary>Nearby say, generic group affirms, combat acknowledge.</summary>
        Normal = 2,
        /// <summary>Group combat callouts, death reactions, loot requests.</summary>
        High = 3,
        /// <summary>Whispers to player, group join/leave, direct player interaction.</summary>
        Critical = 4
    }

    /// <summary>
    /// A single queued paraphrase request with priority and delivery callback.
    /// </summary>
    public class ParaphraseJob
    {
        public string Text;
        public string Trigger;
        public string SimName;
        public string Zone;
        public string Channel;
        public float Relationship;
        public ParaphrasePriority Priority;
        public float EnqueueTime;

        /// <summary>Called with the paraphrased (or original) text result.</summary>
        public Action<string> OnResult;
    }

    /// <summary>
    /// Priority queue that throttles paraphrase requests to avoid overloading
    /// the sidecar/shimmy LLM inference server.
    ///
    /// Design:
    /// - Max 1 concurrent in-flight request (shimmy processes sequentially anyway)
    /// - Queued jobs sorted by priority (highest first), then by age (oldest first)
    /// - Max queue depth -- low priority items are dropped when full
    /// - Stale items (older than TTL) are dropped and delivered as original text
    /// - Skip priority items bypass the queue entirely
    /// </summary>
    public class ParaphraseQueue
    {
        private readonly List<ParaphraseJob> _queue = new List<ParaphraseJob>();
        private readonly EventParaphraser _paraphraser;

        /// <summary>Maximum queued items. New low-priority items are dropped when full.</summary>
        private readonly int _maxQueueSize;

        /// <summary>How long a queued item can wait before being delivered as-is (seconds).</summary>
        private readonly float _staleTtl;

        /// <summary>Max concurrent in-flight paraphrase requests.</summary>
        private readonly int _maxConcurrent;

        private int _inFlight;

        public ParaphraseQueue(EventParaphraser paraphraser,
            int maxQueueSize = 6, int maxConcurrent = 2, float staleTtl = 4f)
        {
            _paraphraser = paraphraser;
            _maxQueueSize = maxQueueSize;
            _maxConcurrent = maxConcurrent;
            _staleTtl = staleTtl;
        }

        /// <summary>
        /// Enqueue a paraphrase job. If priority is Skip, delivers original text
        /// immediately. If queue is full, drops the lowest priority item (or rejects
        /// this one if it's the lowest).
        /// </summary>
        public void Enqueue(ParaphraseJob job)
        {
            // Skip priority: deliver original immediately, no LLM call
            if (job.Priority == ParaphrasePriority.Skip)
            {
                job.OnResult(job.Text);
                return;
            }

            job.EnqueueTime = Time.realtimeSinceStartup;

            // If we have capacity, just add and try to drain
            if (_queue.Count < _maxQueueSize)
            {
                InsertSorted(job);
                TryDrainNext();
                return;
            }

            // Queue is full -- find the lowest priority item
            int lowestIdx = FindLowestPriorityIndex();
            ParaphraseJob lowest = _queue[lowestIdx];

            if (job.Priority <= lowest.Priority)
            {
                // New job is same or lower priority than everything in queue -- drop it
                LogDebug("[ParaphraseQueue] Dropped (queue full, low priority): " +
                    job.SimName + " [" + job.Channel + "] " + Truncate(job.Text, 40));
                job.OnResult(job.Text);
                return;
            }

            // Evict the lowest priority item and deliver its original text
            _queue.RemoveAt(lowestIdx);
            LogDebug("[ParaphraseQueue] Evicted (lower priority): " +
                lowest.SimName + " [" + lowest.Channel + "] " + Truncate(lowest.Text, 40));
            lowest.OnResult(lowest.Text);

            InsertSorted(job);
            TryDrainNext();
        }

        /// <summary>
        /// Called each frame (or periodically) to expire stale items and
        /// kick off pending work if slots are available.
        /// </summary>
        public void Update()
        {
            ExpireStale();
            TryDrainNext();
        }

        /// <summary>Number of items currently queued (not in-flight).</summary>
        public int QueuedCount => _queue.Count;

        /// <summary>Number of in-flight paraphrase requests.</summary>
        public int InFlightCount => _inFlight;

        private void TryDrainNext()
        {
            while (_inFlight < _maxConcurrent && _queue.Count > 0)
            {
                // Take the highest priority item (front of sorted list)
                ParaphraseJob job = _queue[0];
                _queue.RemoveAt(0);

                _inFlight++;

                _paraphraser.ParaphraseText(
                    job.Text, job.Trigger, job.SimName, job.Zone,
                    job.Channel, null, job.Relationship,
                    result =>
                    {
                        _inFlight--;
                        job.OnResult(result);

                        // After completing, try to drain more
                        TryDrainNext();
                    });
            }
        }

        /// <summary>
        /// Remove items that have been waiting too long. Deliver original text.
        /// </summary>
        private void ExpireStale()
        {
            float now = Time.realtimeSinceStartup;
            for (int i = _queue.Count - 1; i >= 0; i--)
            {
                if (now - _queue[i].EnqueueTime > _staleTtl)
                {
                    ParaphraseJob stale = _queue[i];
                    _queue.RemoveAt(i);
                    LogDebug("[ParaphraseQueue] Expired (stale): " +
                        stale.SimName + " [" + stale.Channel + "] " + Truncate(stale.Text, 40));
                    stale.OnResult(stale.Text);
                }
            }
        }

        /// <summary>
        /// Insert job in sorted position: highest priority first,
        /// then oldest first within same priority.
        /// </summary>
        private void InsertSorted(ParaphraseJob job)
        {
            for (int i = 0; i < _queue.Count; i++)
            {
                if (job.Priority > _queue[i].Priority)
                {
                    _queue.Insert(i, job);
                    return;
                }
                // Same priority: newer items go after older (FIFO within priority)
            }
            _queue.Add(job);
        }

        private int FindLowestPriorityIndex()
        {
            int idx = 0;
            for (int i = 1; i < _queue.Count; i++)
            {
                if (_queue[i].Priority < _queue[idx].Priority)
                    idx = i;
                else if (_queue[i].Priority == _queue[idx].Priority &&
                         _queue[i].EnqueueTime < _queue[idx].EnqueueTime)
                    // Same priority: evict the oldest
                    idx = i;
            }
            return idx;
        }

        private static string Truncate(string s, int max)
        {
            if (s == null) return "";
            return s.Length <= max ? s : s.Substring(0, max) + "...";
        }

        private static void LogDebug(string message)
        {
            if (ErenshorLLMDialogPlugin.DebugLogging != null &&
                ErenshorLLMDialogPlugin.DebugLogging.Value == Toggle.On)
            {
                ErenshorLLMDialogPlugin.Log.LogInfo(message);
            }
        }
    }
}
