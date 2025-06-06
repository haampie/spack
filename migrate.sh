#!/bin/sh

set -eu

for pr in 27387 28320 31047 32224 34074 34186 34404 35046 35223 35669 35733 35736 35814 35818 36962 37094 37402 37709 37819 37835 38168 38420 38429 38781 39021 39298 39330 39490 39992 40062 40236 40269 41012 41048 41312 41664 41814 41916 42131 42239 42302 42306 42411 42806 42889 43357 43364 43436 43947 44143 44200 44235 44387 44526 44633 44719 44742 45256 45414 45535 45821 45842 45867 45878 46010 46081 46396 46446 46561 46626 46663 46677 46680 46700 46889 46978 47188 47189 47231 47249 47413 47491 47710 47712 47722 47733 47840 47849 47930 47940 47945 47990 48014 48020 48089 48093 48096 48244 48277 48289 48291 48303 48304 48357 48360 48513 48559 48634 48732 48820 48965 48970 49126 49130 49179 49228 49258 49273 49285 49318 49331 49351 49366 49370 49395 49422 49426 49455 49515 49541 49542 49545 49551 49568 49570 49590 49594 49597 49601 49641 49663 49664 49667 49679 49714 49718 49744 49750 49761 49772 49778 49822 49830 49835 49846 49863 49890 49949 49950 49968 50045 50048 50084 50121 50128 50156 50175 50227 50246 50249 50365 50375 50400 50402 50410 50432 50448 50454 50475 50514 50522 50562 50564 50569 50577 50583 50622 50630 50635 50652 50657 50662 50665 50670 50675 50676 50677 50706 50707 50709 50717 50719 50721 50723 50725 50731 50732 50735 50736 50739 50742 50751 50753 50754 50755 50763 50769 50773 50780 50781 50784 50799 50813 50814 50816 50817 50820 50821 50822 ; do
    # check if the PR is open
    if [ "$(gh pr view "$pr" --repo spack/spack --json state --jq '.state')" != "OPEN" ];
    then
        echo "Pull request #$pr is not open, skipping migration."
        continue
    fi

    # If any of the comments contains the text "This comment was posted automatically",
    # then we should not comment again.
    if gh issue view "$pr" --repo spack/spack --json comments --jq '.comments[].body' | grep -q "This comment was posted automatically"; then
        echo "Pull request #$pr has already been commented on, skipping migration."
        continue
    fi

    # Get the author of the PR
    author="$(gh pr view "$pr" --repo spack/spack --json author --jq '.author.login')"

    gh issue comment "$pr" --repo spack/spack --body "Hi @$author, this package-related pull request can be migrated from \`spack/spack\` to \`spack/spack-packages\` using the [**migrate-pkg-prs utility**](https://github.com/spack/migrate-package-prs).

We encourage authors to run the migration script themselves to preserve author attribution.

You can migrate all your open pull requests at once, following the steps in the [documentation](https://github.com/spack/migrate-package-prs?tab=readme-ov-file#usage-instructions).

---

*This comment was posted automatically.*"

done

