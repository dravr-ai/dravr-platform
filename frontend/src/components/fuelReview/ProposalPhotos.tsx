// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Every photo a fuel proposal's label was read from, in the order sent, each served by the backend as a data URL
// ABOUTME: The reviewer sees what the reader saw — fronts of pack included — to judge a read against the photos

import { useQuery } from '@tanstack/react-query';
import { QUERY_KEYS } from '../../constants/queryKeys';
import { adminApi, type FuelProductProposal, type FuelProposalImageMeta } from '../../services/api/admin';
import { productName } from './fuelReviewModel';

function ProposalPhoto({
  proposal,
  image,
  count,
}: {
  proposal: FuelProductProposal;
  image: FuelProposalImageMeta;
  count: number;
}) {
  const photo = useQuery({
    queryKey: QUERY_KEYS.adminFuelProducts.image(proposal.id, image.position),
    queryFn: () => adminApi.getFuelProposalImage(proposal.id, image.position),
    staleTime: Infinity,
  });
  const caption = `Photo ${image.position + 1} of ${count} · ${image.mime} · ${Math.max(1, Math.round(image.byte_len / 1024))} KB`;
  return (
    <figure className="space-y-1" data-testid={`fuel-proposal-photo-${image.position}`}>
      {photo.isPending && (
        <div className="pierre-spinner h-5 w-5" role="status" aria-label={`Loading photo ${image.position + 1}`} />
      )}
      {photo.isError && (
        <p role="alert" className="text-sm text-error">
          Photo {image.position + 1} could not be loaded.
        </p>
      )}
      {photo.isSuccess && (
        <img
          src={photo.data}
          alt={`Photo ${image.position + 1} of ${productName(proposal)}`}
          className="max-h-80 w-full rounded-lg object-contain bg-surface-container-low"
          data-testid="fuel-proposal-image"
        />
      )}
      <figcaption className="text-xs text-on-surface-variant">{caption}</figcaption>
    </figure>
  );
}

export default function ProposalPhotos({
  proposal,
  images,
}: {
  proposal: FuelProductProposal;
  images: FuelProposalImageMeta[];
}) {
  if (images.length === 0) {
    return (
      <p className="text-sm text-on-surface-variant" data-testid="fuel-proposal-no-photos">
        No photo is stored with this proposal.
      </p>
    );
  }
  return (
    <div className="grid gap-4 sm:grid-cols-2" data-testid="fuel-proposal-photos">
      {images.map((image) => (
        <ProposalPhoto key={image.position} proposal={proposal} image={image} count={images.length} />
      ))}
    </div>
  );
}
