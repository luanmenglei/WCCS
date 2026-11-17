"""Version-pinned fixes for native CLICS duration serialization with removed intervals."""
import hashlib

BASELINE = 'ac13d78ea4991552fea56132183d44469dfd6084e4c0095be02cfa6663c3ad3d'
REPLACEMENTS = {
    'return Utils::relTime($this->getEndtime() - $this->starttime);':
        'return Utils::relTime($this->getContestTime($this->getEndtime()));',
    'return Utils::relTime($this->getEndtime() - $this->getFreezetime());':
        'return Utils::relTime($this->getContestTime($this->getEndtime()) - $this->getContestTime($this->getFreezetime()));',
}


def patched(original: bytes) -> bytes:
    if hashlib.sha256(original).hexdigest() != BASELINE:
        raise ValueError('Contest.php differs from the fixed DOMjudge 9.0.2 source.')
    text = original.decode('utf-8')
    for before, after in REPLACEMENTS.items():
        if text.count(before) != 1:
            raise ValueError('Native timing method does not match the verified source.')
        text = text.replace(before, after)
    return text.encode('utf-8')
