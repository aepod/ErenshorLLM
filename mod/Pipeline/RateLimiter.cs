using System.Collections.Generic;
using UnityEngine;

namespace ErenshorLLMDialog.Pipeline
{
    /// <summary>
    /// Sliding-window rate limiter for sidecar requests.
    /// Tracks request timestamps and enforces a maximum number of requests
    /// within a rolling time window. Uses Time.realtimeSinceStartup for
    /// timestamps (unaffected by Unity time scale).
    /// </summary>
    public class RateLimiter
    {
        private readonly int _maxRequests;
        private readonly float _windowSeconds;
        private readonly Queue<float> _timestamps;

        public RateLimiter(int maxRequests, float windowSeconds)
        {
            _maxRequests = maxRequests;
            _windowSeconds = windowSeconds;
            _timestamps = new Queue<float>();
        }

        /// <summary>
        /// Attempts to consume a request slot. Returns true if the request
        /// is within the rate limit. Records the timestamp on success.
        /// </summary>
        public bool TryConsume()
        {
            PurgeExpired();
            if (_timestamps.Count >= _maxRequests)
                return false;

            _timestamps.Enqueue(Time.realtimeSinceStartup);
            return true;
        }

        /// <summary>
        /// Number of request slots remaining in the current window.
        /// </summary>
        public int Remaining
        {
            get
            {
                PurgeExpired();
                return Mathf.Max(0, _maxRequests - _timestamps.Count);
            }
        }

        private void PurgeExpired()
        {
            float cutoff = Time.realtimeSinceStartup - _windowSeconds;
            while (_timestamps.Count > 0 && _timestamps.Peek() < cutoff)
                _timestamps.Dequeue();
        }
    }
}
