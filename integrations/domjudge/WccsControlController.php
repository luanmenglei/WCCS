<?php declare(strict_types=1);

// SPDX-License-Identifier: AGPL-3.0-or-later
// WCCS extension for the fixed DOMjudge 9.0.2 release.
namespace App\Controller\API;

use App\Entity\Contest;
use App\Entity\RemovedInterval;
use App\Service\DOMJudgeService;
use App\Service\EventLogService;
use App\Utils\Utils;
use Doctrine\ORM\EntityManagerInterface;
use FOS\RestBundle\Controller\Annotations as Rest;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\JsonResponse;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpKernel\Exception\ConflictHttpException;
use Symfony\Component\HttpKernel\Exception\BadRequestHttpException;
use Symfony\Component\Security\Http\Attribute\IsGranted;

#[Rest\Route('/contests/{contestId}/wccs-control')]
class WccsControlController extends AbstractController
{
    // A pending interval remains closed to submissions across WCCS/PHP restarts.
    // Resume replaces this sentinel with the actual end of the pause.
    private const OPEN_INTERVAL_END = 4102444800;
    private const MILESTONES = ['Endtime', 'Freezetime', 'Unfreezetime', 'Deactivatetime'];

    public function __construct(
        private readonly EntityManagerInterface $em,
        private readonly DOMJudgeService $dj,
        private readonly EventLogService $events,
    ) {
    }

    private function snapshot(Contest $contest): array
    {
        $value = ['start' => (float)$contest->getStarttime(false),
            'activate' => $contest->getActivatetime(),
            'start_enabled' => $contest->getStarttimeEnabled(),
            'allow_submit' => $contest->getAllowSubmit()];
        foreach (self::MILESTONES as $field) {
            $time = $contest->{'get' . $field}();
            $value[$field] = $time === null ? null : (float)$time;
        }
        $value['intervals'] = [];
        foreach ($contest->getRemovedIntervals() as $interval) {
            $value['intervals'][] = [$interval->getIntervalid(), (float)$interval->getStarttime(), (float)$interval->getEndtime()];
        }
        usort($value['intervals'], static fn(array $a, array $b): int => $a[0] <=> $b[0]);
        return $value;
    }

    private function record(Contest $contest): ?array
    {
        $row = $this->em->getConnection()->fetchAssociative(
            'SELECT * FROM wccs_contest_control WHERE cid=?', [$contest->getCid()]);
        return $row === false ? null : $row;
    }

    private function contest(string $externalId): Contest
    {
        $contest = $this->em->getRepository(Contest::class)->findOneBy(['externalid' => $externalId]);
        if ($contest === null) {
            throw $this->createNotFoundException('contest_not_found');
        }
        return $contest;
    }

    private function view(Contest $contest, ?array $row): array
    {
        $now = Utils::now();
        $paused = $row !== null && $row['phase'] === 'paused';
        $checkpoint = $row === null ? [] : json_decode($row['checkpoint'], true, flags: JSON_THROW_ON_ERROR);
        $remaining = $paused
            ? (float)$checkpoint['Endtime'] - (float)$row['paused_at']
            : (float)$contest->getEndtime() - $now;
        $phase = $paused ? 'paused' : (!$contest->getStarttimeEnabled() || $now < $contest->getStarttime(false)
            ? 'scheduled' : ($remaining <= 0 ? 'ended' : 'running'));
        return ['extension_version' => 1, 'contest_id' => $contest->getExternalid(),
            'revision' => (int)($row['revision'] ?? 0), 'phase' => $phase,
            'paused' => $paused, 'server_time' => $now,
            'remaining_seconds' => max(0, $remaining),
            'elapsed_seconds' => max(0, $contest->getContestTime($now)),
            'allow_submit' => $contest->getAllowSubmit(),
            'snapshot_hash' => hash('sha256', json_encode($this->snapshot($contest), JSON_THROW_ON_ERROR))];
    }

    #[Rest\Get('', name: 'wccs_control_status')]
    public function status(string $contestId): JsonResponse
    {
        $contest = $this->contest($contestId);
        if (!$this->isGranted('ROLE_API_READER') && !$contest->isActive()) {
            throw $this->createAccessDeniedException();
        }
        return new JsonResponse($this->view($contest, $this->record($contest)), headers: ['Cache-Control' => 'no-store']);
    }

    private function timeString(float $time): string
    {
        return gmdate('Y-m-d H:i:s', (int)$time) . sprintf('.%03d UTC', (int)round(($time - floor($time)) * 1000));
    }

    private function shift(Contest $contest, array $checkpoint, float $from, float $seconds): void
    {
        foreach (self::MILESTONES as $field) {
            $value = $checkpoint[$field];
            if ($value !== null && (float)$value >= $from) {
                $contest->{'set' . $field . 'String'}($this->timeString((float)$value + $seconds));
            }
        }
    }

    private function closePause(Contest $contest, array $row, float $now): void
    {
        $interval = $this->em->getRepository(RemovedInterval::class)->find($row['interval_id']);
        if ($interval === null || $interval->getContest()->getCid() !== $contest->getCid()) {
            throw new ConflictHttpException('pause_interval_missing_review_required');
        }
        $checkpoint = json_decode($row['checkpoint'], true, flags: JSON_THROW_ON_ERROR);
        if (($checkpoint['paused_snapshot_hash'] ?? '') !== hash('sha256', json_encode($this->snapshot($contest), JSON_THROW_ON_ERROR))) {
            throw new ConflictHttpException('paused_contest_changed_review_required');
        }
        $interval->setEndtimeString($this->timeString(max($now, (float)$row['paused_at'] + 0.001)));
        $seconds = (float)$interval->getEndtime() - (float)$row['paused_at'];
        $this->shift($contest, $checkpoint, (float)$row['paused_at'], $seconds);
        $contest->setAllowSubmit($checkpoint['allow_submit']);
    }

    #[Rest\Post('', name: 'wccs_control_write')]
    #[IsGranted('ROLE_API_WRITER')]
    public function write(string $contestId, Request $request): JsonResponse
    {
        $input = $request->toArray();
        if (array_diff(array_keys($input), ['request_id', 'action', 'revision', 'snapshot_hash']) !== []
            || !preg_match('/^[A-Za-z0-9_-]{1,80}$/D', $input['request_id'] ?? '')
            || !in_array($input['action'] ?? '', ['hold_start', 'start', 'pause', 'resume', 'end'], true)
            || !is_int($input['revision'] ?? null)
            || !preg_match('/^[a-f0-9]{64}$/D', $input['snapshot_hash'] ?? '')) {
            throw new BadRequestHttpException('invalid_control_request');
        }
        $contest = $this->contest($contestId);
        $db = $this->em->getConnection();
        $db->beginTransaction();
        try {
            $db->executeStatement('INSERT IGNORE INTO wccs_contest_control(cid,revision,phase,checkpoint) VALUES(?,0,?,?)',
                [$contest->getCid(), 'running', '{}']);
            $db->fetchOne('SELECT cid FROM wccs_contest_control WHERE cid=? FOR UPDATE', [$contest->getCid()]);
            $fingerprint = hash('sha256', json_encode([$contestId, $input], JSON_THROW_ON_ERROR));
            $previous = $db->fetchAssociative('SELECT fingerprint,response FROM wccs_control_requests WHERE request_id=?', [$input['request_id']]);
            if ($previous !== false) {
                if ($previous['fingerprint'] !== $fingerprint) {
                    throw new ConflictHttpException('request_id_reused');
                }
                $db->commit();
                return new JsonResponse(json_decode($previous['response'], true, flags: JSON_THROW_ON_ERROR));
            }
            $this->em->refresh($contest);
            $row = $this->record($contest);
            $before = $this->view($contest, $row);
            if ($before['revision'] !== $input['revision'] || $before['snapshot_hash'] !== $input['snapshot_hash']) {
                throw new ConflictHttpException('contest_changed_review_required');
            }
            $now = floor(Utils::now() * 1000) / 1000;
            $phase = $before['phase'];
            if ($input['action'] === 'hold_start' && $phase === 'scheduled') {
                $contest->setStarttimeEnabled(false);
                $contest->setAllowSubmit(false);
            } elseif ($input['action'] === 'start' && $phase === 'scheduled') {
                if (!$contest->getRemovedIntervals()->isEmpty()) {
                    throw new ConflictHttpException('scheduled_contest_has_removed_intervals');
                }
                $checkpoint = $this->snapshot($contest);
                $contest->setStarttime($now);
                $contest->setStarttimeString($this->timeString($now));
                $contest->setStarttimeEnabled(true);
                $this->shift($contest, $checkpoint, $checkpoint['start'], $now - $checkpoint['start']);
                if ($contest->getActivatetime() > $now) {
                    $contest->setActivatetimeString($this->timeString($now));
                }
                $contest->setAllowSubmit(true);
            } elseif ($input['action'] === 'pause' && $phase === 'running') {
                $checkpoint = $this->snapshot($contest);
                $interval = new RemovedInterval();
                $interval->setContest($contest);
                $interval->setStarttimeString($this->timeString($now));
                $interval->setEndtimeString($this->timeString(self::OPEN_INTERVAL_END));
                $contest->addRemovedInterval($interval);
                $this->em->persist($interval);
                $contest->setAllowSubmit(false);
                $this->shift($contest, $checkpoint, $now, self::OPEN_INTERVAL_END - $now);
                $this->em->flush();
                $checkpoint['paused_snapshot_hash'] = hash('sha256', json_encode($this->snapshot($contest), JSON_THROW_ON_ERROR));
                $db->executeStatement('UPDATE wccs_contest_control SET phase=?,paused_at=?,interval_id=?,checkpoint=? WHERE cid=?',
                    ['paused', $now, $interval->getIntervalid(), json_encode($checkpoint, JSON_THROW_ON_ERROR), $contest->getCid()]);
            } elseif ($input['action'] === 'resume' && $phase === 'paused') {
                $this->closePause($contest, $row, $now);
                $db->executeStatement('UPDATE wccs_contest_control SET phase=? WHERE cid=?', ['running', $contest->getCid()]);
            } elseif ($input['action'] === 'end' && in_array($phase, ['running', 'paused'], true)) {
                if ($phase === 'paused') {
                    $this->closePause($contest, $row, $now);
                }
                $contest->setAllowSubmit(false);
                $contest->setEndtimeString($this->timeString($now));
                $db->executeStatement('UPDATE wccs_contest_control SET phase=? WHERE cid=?', ['ended', $contest->getCid()]);
            } elseif (!(($input['action'] === 'start' && $phase === 'running')
                || ($input['action'] === 'pause' && $phase === 'paused')
                || ($input['action'] === 'resume' && $phase === 'running')
                || ($input['action'] === 'end' && $phase === 'ended'))) {
                throw new ConflictHttpException('contest_phase_incompatible');
            }
            $this->em->flush();
            $db->executeStatement('UPDATE wccs_contest_control SET revision=revision+1 WHERE cid=?', [$contest->getCid()]);
            $this->dj->auditlog('contest', $contest->getCid(), 'wccs_' . $input['action'], $input['request_id']);
            $this->events->log('contest', $contest->getCid(), EventLogService::ACTION_UPDATE, $contest->getCid());
            $response = $this->view($contest, $this->record($contest));
            $db->insert('wccs_control_requests', ['request_id' => $input['request_id'], 'fingerprint' => $fingerprint,
                'response' => json_encode($response, JSON_THROW_ON_ERROR)]);
            $db->commit();
            return new JsonResponse($response);
        } catch (\Throwable $error) {
            $db->rollBack();
            throw $error;
        }
    }
}
